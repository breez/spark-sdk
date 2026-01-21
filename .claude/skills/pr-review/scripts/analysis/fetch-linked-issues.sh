#!/bin/bash
# Fetch linked issues and extract requirements for PR context
# Usage: fetch-linked-issues.sh <pr-number>

set -euo pipefail

PR_NUMBER="${1:-}"

if [[ -z "$PR_NUMBER" ]]; then
  echo "Usage: $0 <pr-number>" >&2
  exit 1
fi

echo "=== Linked Issues Context ==="
echo ""

# Track if we found any issues
FOUND_ISSUES=false

# Method 1: Parse PR body for "Closes #N", "Fixes #N", etc.
echo "Checking PR description for linked issues..."
BODY=$(gh pr view "$PR_NUMBER" --json body -q .body 2>/dev/null || echo "")

if [[ -n "$BODY" ]]; then
  # Extract issue numbers from keywords like "Closes #123", "Fixes #456"
  LINKED_ISSUES=$(echo "$BODY" | grep -oiE "(close[sd]?|fix(e[sd])?|resolve[sd]?) #[0-9]+" | grep -oE "[0-9]+" | sort -u || true)

  if [[ -n "$LINKED_ISSUES" ]]; then
    FOUND_ISSUES=true
    while IFS= read -r issue_num; do
      echo ""
      echo "--- Issue #$issue_num (from PR description) ---"

      # Fetch issue details
      ISSUE_JSON=$(gh issue view "$issue_num" --json title,body,labels,state 2>/dev/null || echo '{}')

      if [[ "$ISSUE_JSON" != "{}" ]]; then
        echo "Title: $(echo "$ISSUE_JSON" | jq -r '.title // "N/A"')"
        echo "State: $(echo "$ISSUE_JSON" | jq -r '.state // "N/A"')"
        echo "Labels: $(echo "$ISSUE_JSON" | jq -r '.labels[]?.name // empty' | paste -sd ',' - || echo "none")"
        echo ""
        echo "Description:"
        echo "$ISSUE_JSON" | jq -r '.body // "No description"' | head -20
        echo ""
      else
        echo "Could not fetch issue details (may be in different repo or deleted)"
      fi
    done <<< "$LINKED_ISSUES"
  fi
fi

# Method 2: Check Development section via GitHub API
echo "Checking Development section for linked issues..."
CLOSING_ISSUES=$(gh api "repos/breez/spark-sdk/pulls/$PR_NUMBER" --jq '.closing_issues_references[]?.number // empty' 2>/dev/null | sort -u || true)

if [[ -n "$CLOSING_ISSUES" ]]; then
  FOUND_ISSUES=true
  while IFS= read -r issue_num; do
    # Skip if already processed from PR body
    if echo "$LINKED_ISSUES" | grep -q "^${issue_num}$" 2>/dev/null; then
      continue
    fi

    echo ""
    echo "--- Issue #$issue_num (from Development section) ---"

    # Fetch issue details
    ISSUE_JSON=$(gh issue view "$issue_num" --json title,body,labels,state 2>/dev/null || echo '{}')

    if [[ "$ISSUE_JSON" != "{}" ]]; then
      echo "Title: $(echo "$ISSUE_JSON" | jq -r '.title // "N/A"')"
      echo "State: $(echo "$ISSUE_JSON" | jq -r '.state // "N/A"')"
      echo "Labels: $(echo "$ISSUE_JSON" | jq -r '.labels[]?.name // empty' | paste -sd ',' - || echo "none")"
      echo ""
      echo "Description:"
      echo "$ISSUE_JSON" | jq -r '.body // "No description"' | head -20
      echo ""
    else
      echo "Could not fetch issue details"
    fi
  done <<< "$CLOSING_ISSUES"
fi

if [[ "$FOUND_ISSUES" == "false" ]]; then
  echo "No linked issues found."
  echo ""
fi

echo "=== End Linked Issues ==="
