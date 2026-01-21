#!/bin/bash
# Add a comment to an existing PR review (or create COMMENT-only review)
# Use this instead of posting a duplicate REQUEST_CHANGES review
#
# Usage: add-review-comment.sh <pr-number> <event> <message>
#
# <event>: COMMENT (non-blocking) or REQUEST_CHANGES (blocking)
# <message>: Your comment text (can be multi-line)
#
# Example:
#   add-review-comment.sh 569 COMMENT "Update: Issue #1 still present in latest commit"
#   add-review-comment.sh 569 COMMENT "New finding: Missing validation on line 42"

set -euo pipefail

PR_NUMBER="${1:-}"
EVENT="${2:-}"
MESSAGE="${3:-}"

if [[ -z "$PR_NUMBER" || -z "$EVENT" || -z "$MESSAGE" ]]; then
  echo "Usage: $0 <pr-number> <event> <message>" >&2
  echo "" >&2
  echo "<event> options:" >&2
  echo "  COMMENT       - Non-blocking feedback" >&2
  echo "  REQUEST_CHANGES - Blocking feedback (use sparingly; usually for new issues)" >&2
  exit 1
fi

# Validate event type
case "$EVENT" in
  COMMENT|REQUEST_CHANGES)
    ;;
  *)
    echo "❌ Invalid event: $EVENT" >&2
    echo "   Must be: COMMENT or REQUEST_CHANGES" >&2
    exit 1
    ;;
esac

# Get current commit
COMMIT_SHA=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid)

# Format message with context
FULL_MESSAGE=$(cat <<EOF
$MESSAGE

---
_Posted as follow-up to previous review_
EOF
)

echo "Posting $EVENT review to PR #$PR_NUMBER..."

# Submit review
gh api "repos/breez/spark-sdk/pulls/$PR_NUMBER/reviews" \
  --method POST \
  --input - <<EOF
{
  "body": "$FULL_MESSAGE",
  "event": "$EVENT",
  "commit_id": "$COMMIT_SHA"
}
EOF

echo "✅ Review posted successfully"
