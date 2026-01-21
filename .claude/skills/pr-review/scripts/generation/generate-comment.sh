#!/bin/bash
# Generate a properly formatted inline comment for GitHub PR reviews
# Usage: generate-comment.sh <path> <start-line> <end-line> <severity> <body>
#
# Parameters:
#   path       - File path relative to repo root (e.g., "crates/foo/src/lib.rs")
#   start-line - First line of the range (must be positive integer)
#   end-line   - Last line of the range (must be > start-line)
#   severity   - One of: Blocking, Important, Suggestion
#   body       - Comment text (can be multi-line, will be JSON-escaped)
#
# Example:
#   generate-comment.sh "crates/sdk/src/lib.rs" 42 45 "Blocking" "SQL injection on line 43"
#
# Output: Writes JSON object to stdout, suitable for jq array construction

set -euo pipefail

PATH_ARG="${1:-}"
START_LINE="${2:-}"
END_LINE="${3:-}"
SEVERITY="${4:-}"
BODY="${5:-}"

# Validate inputs
if [[ -z "$PATH_ARG" || -z "$START_LINE" || -z "$END_LINE" || -z "$SEVERITY" || -z "$BODY" ]]; then
  cat >&2 << 'EOF'
Usage: generate-comment.sh <path> <start-line> <end-line> <severity> <body>

Parameters:
  path       - File path relative to repo root
  start-line - First line of comment range (positive integer)
  end-line   - Last line of comment range (must be > start-line)
  severity   - One of: Blocking, Important, Suggestion
  body       - Comment text

Example:
  generate-comment.sh "src/main.rs" 10 15 "Blocking" "Security issue on line 12"
EOF
  exit 1
fi

# Validate line numbers are integers
if ! [[ "$START_LINE" =~ ^[0-9]+$ ]]; then
  echo "❌ start-line must be a positive integer, got: $START_LINE" >&2
  exit 1
fi

if ! [[ "$END_LINE" =~ ^[0-9]+$ ]]; then
  echo "❌ end-line must be a positive integer, got: $END_LINE" >&2
  exit 1
fi

# Validate start_line < end_line
if [[ "$START_LINE" -ge "$END_LINE" ]]; then
  echo "❌ start-line ($START_LINE) must be less than end-line ($END_LINE)" >&2
  exit 1
fi

# Validate severity
case "$SEVERITY" in
  Blocking|Important|Suggestion)
    ;;
  *)
    echo "❌ severity must be one of: Blocking, Important, Suggestion" >&2
    exit 1
    ;;
esac

# Generate the JSON object using jq (handles proper escaping)
jq -n \
  --arg path "$PATH_ARG" \
  --argjson start_line "$START_LINE" \
  --argjson line "$END_LINE" \
  --arg side "RIGHT" \
  --arg start_side "RIGHT" \
  --arg body "$BODY" \
  '{
    path: $path,
    start_line: $start_line,
    line: $line,
    side: $side,
    start_side: $start_side,
    body: $body
  }'
