#!/bin/bash
# Post a PR review with inline comments to GitHub
# Usage: post-review.sh <pr-number> <event> <summary> [comments-json-file] [model-name]
#
# <event>: COMMENT, APPROVE, or REQUEST_CHANGES
# [comments-json-file]: Optional path to a JSON file containing an array of comments
# [model-name]: Optional model name for attribution (e.g., "sonnet-4.5", "opus-4.5", "haiku-4")
#
# Environment variables for cost tracking (optional, for CLI display only):
#   REVIEW_START_TIME: Unix timestamp when review started
#   REVIEW_INPUT_TOKENS: Total input tokens used
#   REVIEW_OUTPUT_TOKENS: Total output tokens used
#
# IMPORTANT: Always use multi-line ranges (start_line + line) for proper display in GitHub UI.

set -euo pipefail

PR_NUMBER="${1:-}"
EVENT="${2:-}"
SUMMARY="${3:-}"
COMMENTS_FILE="${4:-}"
MODEL_NAME="${5:-sonnet-4.5}"

if [[ -z "$PR_NUMBER" || -z "$EVENT" || -z "$SUMMARY" ]]; then
  echo "Usage: $0 <pr-number> <event> <summary> [comments-json-file]" >&2
  exit 1
fi

# 1. Get the latest commit SHA for the PR
COMMIT_SHA=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid)

# 2. Format model name for display
MODEL_DISPLAY_NAME=""
case "$MODEL_NAME" in
  sonnet-4.5|sonnet)
    MODEL_DISPLAY_NAME="Claude Sonnet 4.5"
    ;;
  opus-4.5|opus)
    MODEL_DISPLAY_NAME="Claude Opus 4.5"
    ;;
  haiku-4.5|haiku)
    MODEL_DISPLAY_NAME="Claude Haiku 4.5"
    ;;
  *)
    MODEL_DISPLAY_NAME="Claude $MODEL_NAME"
    ;;
esac

# 3. Format the review body with experimental disclaimer and model attribution
FULL_BODY=$(cat <<EOF
> 🧪 Experimental PR review using Claude Code.

---

$SUMMARY

**Recommendation:** $EVENT

---
*Reviewed by $MODEL_DISPLAY_NAME*
EOF
)

# 3. Construct the JSON payload using jq
# This ensures correct structure and escaping
PAYLOAD=$(jq -n \
  --arg body "$FULL_BODY" \
  --arg event "$EVENT" \
  --arg commit_id "$COMMIT_SHA" \
  --argjson comments "${COMMENTS_DATA:-[]}" \
  '{body: $body, event: $event, commit_id: $commit_id, comments: $comments}')

# If a comments file was provided, load it into the payload
if [[ -n "$COMMENTS_FILE" && -f "$COMMENTS_FILE" ]]; then
  PAYLOAD=$(jq --argjson c "$(cat "$COMMENTS_FILE")" '.comments = $c' <<< "$PAYLOAD")
fi

# 4. Submit the review
gh api "repos/breez/spark-sdk/pulls/$PR_NUMBER/reviews" \
  --method POST \
  --input - <<< "$PAYLOAD"

# 5. Display efficiency metrics
echo "" >&2
echo "=== Review Efficiency Metrics ===" >&2

# Cost and duration tracking (if provided via environment)
if [[ -n "${REVIEW_START_TIME:-}" ]]; then
  END_TIME=$(date +%s)
  DURATION=$((END_TIME - REVIEW_START_TIME))
  echo "Duration: ${DURATION}s" >&2
fi

if [[ -n "${REVIEW_INPUT_TOKENS:-}" && -n "${REVIEW_OUTPUT_TOKENS:-}" ]]; then
  # Calculate cost based on model pricing
  # Source: https://platform.claude.com/docs/en/about-claude/pricing (Jan 2025)
  INPUT_TOKENS=${REVIEW_INPUT_TOKENS}
  OUTPUT_TOKENS=${REVIEW_OUTPUT_TOKENS}

  case "$MODEL_NAME" in
    sonnet-4.5|sonnet)
      # $3/MTok input, $15/MTok output
      COST=$(awk "BEGIN {printf \"%.4f\", ($INPUT_TOKENS * 3.0 / 1000000) + ($OUTPUT_TOKENS * 15.0 / 1000000)}")
      ;;
    opus-4.5|opus)
      # $5/MTok input, $25/MTok output
      COST=$(awk "BEGIN {printf \"%.4f\", ($INPUT_TOKENS * 5.0 / 1000000) + ($OUTPUT_TOKENS * 25.0 / 1000000)}")
      ;;
    haiku-4.5|haiku)
      # $1/MTok input, $5/MTok output
      COST=$(awk "BEGIN {printf \"%.4f\", ($INPUT_TOKENS * 1.0 / 1000000) + ($OUTPUT_TOKENS * 5.0 / 1000000)}")
      ;;
    *)
      COST="N/A"
      ;;
  esac

  if [[ "$COST" != "N/A" ]]; then
    echo "Cost: \$$COST" >&2
    echo "Tokens: $INPUT_TOKENS input + $OUTPUT_TOKENS output = $((INPUT_TOKENS + OUTPUT_TOKENS)) total" >&2
  fi
fi

# Combined cost/duration display (Claude Code Reports style)
if [[ -n "${REVIEW_START_TIME:-}" && -n "${REVIEW_INPUT_TOKENS:-}" && "$COST" != "N/A" ]]; then
  echo "" >&2
  echo "Cost: \$$COST | Duration: ${DURATION}s" >&2
  echo "" >&2
fi

# Count cached files
CACHE_DIR="/tmp/pr-review-$PR_NUMBER-$COMMIT_SHA"
if [[ -d "$CACHE_DIR" ]]; then
  CACHED_FILES=$(ls -1 "$CACHE_DIR" 2>/dev/null | wc -l | tr -d ' ')
  echo "Cached files: $CACHED_FILES" >&2

  # Estimate API calls saved (each cached file saves ~1-2 API calls)
  API_CALLS_SAVED=$((CACHED_FILES * 2))
  echo "Estimated API calls saved by caching: ~$API_CALLS_SAVED" >&2

  # Show cache location for cleanup
  CACHE_SIZE=$(du -sh "$CACHE_DIR" 2>/dev/null | cut -f1)
  echo "Cache size: $CACHE_SIZE at $CACHE_DIR" >&2
else
  echo "No cache directory found (caching may not have been used)" >&2
fi

# Count inline comments posted
if [[ -n "$COMMENTS_FILE" && -f "$COMMENTS_FILE" ]]; then
  COMMENT_COUNT=$(jq 'length' "$COMMENTS_FILE" 2>/dev/null || echo "0")
  echo "Inline comments posted: $COMMENT_COUNT" >&2
fi

echo "===============================" >&2
