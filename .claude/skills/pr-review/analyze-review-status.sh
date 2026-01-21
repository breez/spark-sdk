#!/bin/bash
# Analyze existing review vs current PR state to detect:
# 1. Issues that have been fixed since last review
# 2. Issues that are still present
# 3. New issues not caught by previous review
#
# Usage: analyze-review-status.sh <pr-number> [review-commit-sha]
#
# If review-commit-sha is not provided, uses the most recent REQUEST_CHANGES review

set -euo pipefail

PR_NUMBER="${1:-}"
REVIEW_COMMIT="${2:-}"

if [[ -z "$PR_NUMBER" ]]; then
  echo "Usage: $0 <pr-number> [review-commit-sha]" >&2
  exit 1
fi

echo "=== Analyzing PR #$PR_NUMBER ==="
echo ""

# Get current commit on PR
CURRENT_COMMIT=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid)
echo "Current commit: $CURRENT_COMMIT"

# If no review commit provided, find the most recent REQUEST_CHANGES review
if [[ -z "$REVIEW_COMMIT" ]]; then
  REVIEW_COMMIT=$(gh api repos/breez/spark-sdk/pulls/"$PR_NUMBER"/reviews \
    --jq '.[] | select(.state == "CHANGES_REQUESTED") | .commit_id' | head -1 || echo "")

  if [[ -z "$REVIEW_COMMIT" ]]; then
    echo "❌ No REQUEST_CHANGES review found. Cannot analyze."
    exit 1
  fi
fi

echo "Review commit:  $REVIEW_COMMIT"
echo ""

# Check if commits differ
if [[ "$CURRENT_COMMIT" == "$REVIEW_COMMIT" ]]; then
  echo "⚠️  WARNING: PR has not been updated since the review!"
  echo "   The code reviewed ($REVIEW_COMMIT) is the same as current code."
  echo ""
  echo "This means:"
  echo "  - Issues flagged in the review have NOT been fixed"
  echo "  - Posting a duplicate review would be redundant"
  echo ""
  echo "Next steps:"
  echo "  1. Check if the author is working on fixes"
  echo "  2. If issues are still relevant, add more detail to existing review"
  echo "  3. Or wait for the author to push fixes"
else
  echo "✅ Code has been updated since review"
  echo "   Need to check which issues have been fixed..."
  echo ""
fi

# Extract key issues from last REQUEST_CHANGES review
echo "=== Issues from Previous Review ==="
REVIEW_BODY=$(gh api repos/breez/spark-sdk/pulls/"$PR_NUMBER"/reviews \
  --jq '.[] | select(.state == "CHANGES_REQUESTED") | .body' | head -1)

echo "$REVIEW_BODY" | head -20
echo ""

# Show inline comments from the review
echo "=== Inline Comments on Review ==="
INLINE_COMMENTS=$(gh api repos/breez/spark-sdk/pulls/"$PR_NUMBER"/comments \
  --jq '.[] | select(.commit_id == "'$REVIEW_COMMIT'") | {path, line, body: (.body | split("\n") | .[0])}' | head -5)

if [[ -n "$INLINE_COMMENTS" ]]; then
  echo "$INLINE_COMMENTS" | jq -r '"  \(.path):\(.line) - \(.body[0:60])"'
else
  echo "  (No inline comments found)"
fi

echo ""
echo "=== Recommendations ==="

if [[ "$CURRENT_COMMIT" == "$REVIEW_COMMIT" ]]; then
  cat << 'EOF'
Since the code hasn't changed:

DUPLICATE RISK: HIGH
  - Do NOT post another REQUEST_CHANGES review
  - Do NOT post identical comments

OPTIONS:
  1. WAIT - Author may still be working on fixes

  2. ADD COMMENTS - If you found NEW issues not in previous review:
     Use: add-review-comment.sh <pr-number> <issue-details>

  3. FOLLOW UP - If no progress after a reasonable time:
     gh pr comment <pr-number> -b "Checking on status of fixes..."
EOF
else
  cat << 'EOF'
Since code HAS changed, need to verify:

NEXT STEPS:
  1. Re-examine previously flagged issues to see if they're FIXED
  2. Look for any NEW issues introduced in the changes
  3. Check if the fix is COMPLETE or PARTIAL

ACTION OPTIONS:
  a) All issues FIXED → Approve the PR or Comment "Ready to merge"
  b) Some FIXED, some remain → Post COMMENT review with updated status
  c) New issues found → Post REQUEST_CHANGES with new findings
  d) Issues partially fixed → Post COMMENT requesting completion

Run a detailed review with:
  make check              # Verify tests still pass
  git diff $REVIEW_COMMIT..HEAD  # See what changed
EOF
fi
