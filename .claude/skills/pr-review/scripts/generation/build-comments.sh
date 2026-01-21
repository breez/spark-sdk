#!/bin/bash
# Build a properly formatted comments JSON array for PR reviews
# Reads comment specifications from stdin and outputs a JSON array
#
# Input format (one comment per line):
#   path:start:end:severity:body
#
# Example input:
#   crates/sdk/src/lib.rs:42:45:Blocking:SQL injection vulnerability
#   crates/sdk/src/lib.rs:100:105:Important:Error handling issue
#
# Output: JSON array suitable for post-review.sh
#
# Usage:
#   cat comments.txt | build-comments.sh > comments.json
#   ./build-comments.sh < comments.txt > comments.json

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Start with empty array
echo "["

FIRST=true
COMMENT_COUNT=0

while IFS= read -r line; do
  # Skip empty lines and comments
  [[ -z "$line" || "$line" =~ ^# ]] && continue

  # Parse the line using colon delimiter
  # Format: path:start:end:severity:body
  IFS=':' read -r path start end severity rest <<< "$line"

  # Everything after the 5th colon is the body (allows colons in body text)
  body="$rest"
  for i in {6..10}; do
    if IFS=':' read -r -a parts <<< "$line"; then
      if [[ ${#parts[@]} -gt 5 ]]; then
        body="${parts[*]:4}"
        break
      fi
    fi
  done

  # Use generate-comment.sh for validation and formatting
  if COMMENT=$("$SCRIPT_DIR/generate-comment.sh" "$path" "$start" "$end" "$severity" "$body" 2>&1); then
    if [[ "$FIRST" == "true" ]]; then
      FIRST=false
    else
      echo ","
    fi
    echo -n "  $COMMENT"
    COMMENT_COUNT=$((COMMENT_COUNT + 1))
  else
    echo "❌ Failed to generate comment from line: $line" >&2
    echo "   Error: $COMMENT" >&2
    exit 1
  fi
done

echo ""
echo "]"

if [[ "$COMMENT_COUNT" -eq 0 ]]; then
  echo "⚠️  Warning: No comments generated" >&2
fi
