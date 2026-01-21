#!/bin/bash
# Filter PR diff to exclude auto-generated files
# Usage: filter-diff.sh <pr-number> [OPTIONS]
#
# Options:
#   --names-only        Show only filenames (default: full diff)
#   --summary          Show brief overview of changes
#   --file <path>      Show diff for specific file only
#   --stats            Show change statistics per file

# ==============================================================================
# IGNORE PATTERNS - Add new patterns here (one per line)
# ==============================================================================
IGNORE_PATTERNS=(
  'Cargo\.lock$'        # Rust lockfile
  '\.lock$'             # Other lockfiles
  '\.generated\.'       # Generated code markers
  'frb_generated\.'     # Flutter Rust Bridge generated
)
# ==============================================================================

set -euo pipefail

# Join array into regex pattern
IGNORE_REGEX=$(IFS='|'; echo "${IGNORE_PATTERNS[*]}")

PR_NUMBER="${1:-}"
MODE="${2:-full}"
SPECIFIC_FILE="${3:-}"

if [ -z "$PR_NUMBER" ]; then
  echo "Usage: filter-diff.sh <pr-number> [OPTIONS]" >&2
  echo "" >&2
  echo "Options:" >&2
  echo "  --names-only        Show only filenames" >&2
  echo "  --summary          Show brief overview" >&2
  echo "  --file <path>      Show diff for specific file" >&2
  echo "  --stats            Show change statistics" >&2
  exit 1
fi

# Get list of non-ignored files
get_filtered_files() {
  gh pr diff "$PR_NUMBER" --name-only | grep -vE "$IGNORE_REGEX"
}

# Handle different modes
case "$MODE" in
  --names-only)
    get_filtered_files
    ;;

  --summary)
    echo "=== PR #$PR_NUMBER Summary ==="
    TOTAL_FILES=$(get_filtered_files | wc -l | tr -d ' ')
    echo "Files changed (excluding auto-generated): $TOTAL_FILES"
    echo ""
    echo "Top changed files:"
    get_filtered_files | head -10
    if [ "$TOTAL_FILES" -gt 10 ]; then
      echo "... and $((TOTAL_FILES - 10)) more files"
    fi
    ;;

  --stats)
    echo "=== Change Statistics ==="
    # Use git diff-tree for accurate line counts
    COMMIT_SHA=$(gh pr view "$PR_NUMBER" --json headRefOid -q .headRefOid)
    BASE_SHA=$(gh pr view "$PR_NUMBER" --json baseRefOid -q .baseRefOid)

    get_filtered_files | while IFS= read -r file; do
      # Count additions and deletions in the diff
      CHANGES=$(gh pr diff "$PR_NUMBER" 2>/dev/null | awk -v f="$file" '
        /^diff --git/ {
          current_file = $3
          sub(/^a\//, "", current_file)
          in_file = (current_file == f)
        }
        in_file && /^\+[^+]/ { adds++ }
        in_file && /^-[^-]/ { dels++ }
        END { print adds+dels }
      ')
      [ -n "$CHANGES" ] && [ "$CHANGES" -gt 0 ] && echo "$CHANGES changes - $file"
    done | sort -rn | head -20
    ;;

  --file)
    if [ -z "$SPECIFIC_FILE" ]; then
      echo "Error: --file requires a file path" >&2
      exit 1
    fi
    # Check if file is in ignore list
    if echo "$SPECIFIC_FILE" | grep -qE "$IGNORE_REGEX"; then
      echo "Warning: File matches ignore pattern" >&2
    fi
    # gh pr diff doesn't support file filtering, so we filter the full diff
    gh pr diff "$PR_NUMBER" | awk -v target="$SPECIFIC_FILE" '
      /^diff --git/ {
        file = $3
        sub(/^a\//, "", file)
        print_file = (file == target)
      }
      print_file { print }
    '
    ;;

  *)
    # Default: full filtered diff
    # Get full diff and filter out ignored files
    gh pr diff "$PR_NUMBER" | awk '
      BEGIN { print_current = 1 }
      /^diff --git/ {
        # Extract filename from "diff --git a/path b/path"
        file = $3
        sub(/^a\//, "", file)

        # Check if file matches ignore patterns
        ignore = 0
        split("'"$IGNORE_REGEX"'", patterns, "|")
        for (i in patterns) {
          if (file ~ patterns[i]) {
            ignore = 1
            break
          }
        }
        print_current = !ignore
      }
      print_current { print }
    '
    ;;
esac
