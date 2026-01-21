#!/bin/bash
# Intelligently chunk large PR diffs based on token limits
# Usage: chunk-diff.sh <pr-number> [--max-tokens N] [--output-dir DIR]
#
# Splits diff into manageable chunks by:
# 1. Keeping complete files together when possible
# 2. Splitting large files at logical boundaries (functions, structs)
# 3. Respecting token limits (~4 chars = 1 token)
#
# Examples:
#   chunk-diff.sh 569
#   chunk-diff.sh 569 --max-tokens 20000
#   chunk-diff.sh 569 --max-tokens 20000 --output-dir /tmp/pr-569-chunks

set -euo pipefail

# Default values
PR_NUMBER=""
MAX_TOKENS=25000
OUTPUT_DIR=""

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --max-tokens)
      MAX_TOKENS="$2"
      shift 2
      ;;
    --output-dir)
      OUTPUT_DIR="$2"
      shift 2
      ;;
    *)
      if [[ -z "$PR_NUMBER" ]]; then
        PR_NUMBER="$1"
      fi
      shift
      ;;
  esac
done

# Set output directory default after parsing PR number
if [[ -z "$OUTPUT_DIR" ]]; then
  OUTPUT_DIR="/tmp/pr-$PR_NUMBER-chunks"
fi

if [[ -z "$PR_NUMBER" ]]; then
  echo "Usage: chunk-diff.sh <pr-number> [--max-tokens N] [--output-dir DIR]" >&2
  exit 1
fi

# Create output directory
mkdir -p "$OUTPUT_DIR"

# Token estimation: ~4 characters = 1 token
MAX_CHARS=$((MAX_TOKENS * 4))

echo "Chunking diff for PR #$PR_NUMBER..." >&2
echo "Max tokens per chunk: $MAX_TOKENS (~$MAX_CHARS chars)" >&2
echo "Output directory: $OUTPUT_DIR" >&2
echo "" >&2

# Get filtered diff
FULL_DIFF_FILE="$OUTPUT_DIR/full-diff.txt"
.claude/skills/pr-review/scripts/fetching/filter-diff.sh "$PR_NUMBER" > "$FULL_DIFF_FILE"

TOTAL_SIZE=$(wc -c < "$FULL_DIFF_FILE")
echo "Total diff size: $TOTAL_SIZE chars" >&2

# Check if chunking is needed
if [[ $TOTAL_SIZE -le $MAX_CHARS ]]; then
  echo "Diff fits in single chunk, no splitting needed" >&2
  cp "$FULL_DIFF_FILE" "$OUTPUT_DIR/chunk-001.diff"
  echo "Created: chunk-001.diff" >&2
  exit 0
fi

# Split diff into file-based chunks
echo "Splitting diff into chunks..." >&2

# Extract file boundaries from diff
awk '/^diff --git/ {print NR}' "$FULL_DIFF_FILE" > "$OUTPUT_DIR/file-boundaries.txt"
echo "$(wc -l < "$FULL_DIFF_FILE")" >> "$OUTPUT_DIR/file-boundaries.txt"

CHUNK_NUM=1
CHUNK_SIZE=0
CHUNK_FILE="$OUTPUT_DIR/chunk-$(printf '%03d' $CHUNK_NUM).diff"
> "$CHUNK_FILE"
CHUNK_FILE_COUNT=0

# Read file boundaries and split accordingly
PREV_LINE=0
while IFS= read -r line_num; do
  if [[ $PREV_LINE -eq 0 ]]; then
    PREV_LINE=$line_num
    continue
  fi

  # Extract this file's diff section
  FILE_DIFF=$(sed -n "${PREV_LINE},${line_num}p" "$FULL_DIFF_FILE")
  FILE_SIZE=${#FILE_DIFF}

  # Check if adding this file would exceed chunk limit
  if [[ $((CHUNK_SIZE + FILE_SIZE)) -gt $MAX_CHARS ]] && [[ $CHUNK_SIZE -gt 0 ]]; then
    # Finish current chunk
    echo "  chunk-$(printf '%03d' $CHUNK_NUM).diff ($CHUNK_SIZE chars, $CHUNK_FILE_COUNT files)" >&2
    ((CHUNK_NUM++))
    CHUNK_FILE="$OUTPUT_DIR/chunk-$(printf '%03d' $CHUNK_NUM).diff"
    > "$CHUNK_FILE"
    CHUNK_SIZE=0
    CHUNK_FILE_COUNT=0
  fi

  # Add file to current chunk
  echo "$FILE_DIFF" >> "$CHUNK_FILE"
  CHUNK_SIZE=$((CHUNK_SIZE + FILE_SIZE))
  ((CHUNK_FILE_COUNT++))

  PREV_LINE=$line_num
done < "$OUTPUT_DIR/file-boundaries.txt"

# Report last chunk
if [[ $CHUNK_SIZE -gt 0 ]]; then
  echo "  chunk-$(printf '%03d' $CHUNK_NUM).diff ($CHUNK_SIZE chars, $CHUNK_FILE_COUNT files)" >&2
fi

# Cleanup temp file
rm -f "$OUTPUT_DIR/file-boundaries.txt"

echo "" >&2
echo "=== Chunking Summary ===" >&2
echo "Total chunks created: $CHUNK_NUM" >&2
echo "Chunks location: $OUTPUT_DIR" >&2
echo "========================" >&2

# Create manifest file
cat > "$OUTPUT_DIR/manifest.txt" << EOF
PR #$PR_NUMBER Diff Chunks
Generated: $(date)
Max tokens per chunk: $MAX_TOKENS (~$MAX_CHARS chars)
Total chunks: $CHUNK_NUM

Chunk files:
EOF

ls -lh "$OUTPUT_DIR"/chunk-*.diff | awk '{print "  " $9 " (" $5 ")"}' >> "$OUTPUT_DIR/manifest.txt"

echo "Manifest written to: $OUTPUT_DIR/manifest.txt" >&2
