#!/bin/bash
# Batch fetch multiple files from a PR with parallel processing and caching
# Usage: batch-fetch-files.sh <pr-number> <file1> [file2] [file3] ...
#        cat files.txt | batch-fetch-files.sh <pr-number> --stdin
#
# Examples:
#   batch-fetch-files.sh 569 core/src/sdk.rs core/src/error.rs
#   echo -e "core/src/sdk.rs\ncore/src/error.rs" | batch-fetch-files.sh 569 --stdin
#
# Output: Creates cache entries for all files, reports statistics

set -euo pipefail

PR_NUMBER="${1:-}"
shift || true

if [[ -z "$PR_NUMBER" ]]; then
  echo "Usage: batch-fetch-files.sh <pr-number> <file1> [file2] ..." >&2
  echo "   or: cat files.txt | batch-fetch-files.sh <pr-number> --stdin" >&2
  exit 1
fi

# Get commit SHA for cache key
COMMIT_SHA=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid 2>/dev/null)
if [[ -z "$COMMIT_SHA" ]]; then
  echo "Error: Could not fetch PR #$PR_NUMBER" >&2
  exit 1
fi

CACHE_DIR="/tmp/pr-review-$PR_NUMBER-$COMMIT_SHA"
mkdir -p "$CACHE_DIR"

# Collect files to fetch
declare -a FILES=()

if [[ "${1:-}" == "--stdin" ]]; then
  # Read from stdin
  while IFS= read -r file; do
    [[ -n "$file" ]] && FILES+=("$file")
  done
else
  # Read from arguments
  FILES=("$@")
fi

if [[ ${#FILES[@]} -eq 0 ]]; then
  echo "Error: No files specified" >&2
  exit 1
fi

echo "Batch fetching ${#FILES[@]} files for PR #$PR_NUMBER..." >&2

# Statistics
ALREADY_CACHED=0
NEWLY_FETCHED=0
FAILED=0

# Fetch each file (use cache if available)
for file in "${FILES[@]}"; do
  # Create cache key
  CACHE_KEY=$(echo "$file" | sed 's/[^a-zA-Z0-9._-]/_/g')
  CACHE_FILE="$CACHE_DIR/$CACHE_KEY"

  if [[ -f "$CACHE_FILE" ]]; then
    # Already cached
    ((ALREADY_CACHED++))
    echo "  ✓ $file (cached)" >&2
  else
    # Fetch from GitHub API
    if gh api "repos/breez/spark-sdk/contents/$file?ref=$COMMIT_SHA" 2>/dev/null \
       | jq -r '.content' | base64 -d > "$CACHE_FILE" 2>/dev/null; then
      ((NEWLY_FETCHED++))
      echo "  ↓ $file (fetched)" >&2
    else
      ((FAILED++))
      echo "  ✗ $file (failed)" >&2
      rm -f "$CACHE_FILE"  # Clean up partial file
    fi
  fi
done

echo "" >&2
echo "=== Batch Fetch Summary ===" >&2
echo "Already cached: $ALREADY_CACHED" >&2
echo "Newly fetched:  $NEWLY_FETCHED" >&2
echo "Failed:         $FAILED" >&2
echo "Total cached files: $(ls -1 "$CACHE_DIR" 2>/dev/null | wc -l | tr -d ' ')" >&2
echo "Cache location: $CACHE_DIR" >&2
echo "===========================" >&2
