#!/bin/bash
# Cached file fetching for PR reviews
# Usage: fetch-file.sh <pr-number> <file-path> [with-line-numbers]
#
# Examples:
#   fetch-file.sh 569 crates/breez-sdk/core/src/sdk.rs
#   fetch-file.sh 569 crates/breez-sdk/core/src/sdk.rs true

set -euo pipefail

PR_NUMBER="${1:-}"
FILE_PATH="${2:-}"
WITH_LINE_NUMBERS="${3:-false}"

if [[ -z "$PR_NUMBER" || -z "$FILE_PATH" ]]; then
  echo "Usage: $0 <pr-number> <file-path> [with-line-numbers]" >&2
  echo "" >&2
  echo "Examples:" >&2
  echo "  $0 569 crates/breez-sdk/core/src/sdk.rs" >&2
  echo "  $0 569 crates/breez-sdk/core/src/sdk.rs true" >&2
  exit 1
fi

# Get commit SHA for cache key
COMMIT_SHA=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid 2>/dev/null)

if [[ -z "$COMMIT_SHA" ]]; then
  echo "Error: Could not fetch PR #$PR_NUMBER" >&2
  exit 1
fi

# Set up cache directory
CACHE_DIR="/tmp/pr-review-$PR_NUMBER-$COMMIT_SHA"
mkdir -p "$CACHE_DIR"

# Create safe cache key from file path
CACHE_KEY=$(echo "$FILE_PATH" | sed 's/[^a-zA-Z0-9._-]/_/g')
CACHE_FILE="$CACHE_DIR/$CACHE_KEY"

# Fetch file if not in cache
if [[ ! -f "$CACHE_FILE" ]]; then
  # Try to fetch from GitHub API
  if ! gh api "repos/breez/spark-sdk/contents/$FILE_PATH?ref=$COMMIT_SHA" 2>/dev/null | jq -r '.content' | base64 -d > "$CACHE_FILE" 2>/dev/null; then
    echo "Error: Could not fetch file '$FILE_PATH' at commit $COMMIT_SHA" >&2
    rm -f "$CACHE_FILE"  # Clean up partial file
    exit 1
  fi
fi

# Output with or without line numbers
if [[ "$WITH_LINE_NUMBERS" == "true" ]]; then
  nl -ba "$CACHE_FILE"
else
  cat "$CACHE_FILE"
fi
