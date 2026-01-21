#!/bin/bash
# Calculate review cost based on model and token usage
# Usage: calculate-cost.sh <model> <input-tokens> <output-tokens>
#
# Models: sonnet-4.5, opus-4.5, haiku-4.5
# Pricing from https://platform.claude.com/docs/en/about-claude/pricing (Jan 2025):
#   Sonnet 4.5: $3/MTok input, $15/MTok output
#   Opus 4.5: $5/MTok input, $25/MTok output
#   Haiku 4.5: $1/MTok input, $5/MTok output

set -euo pipefail

MODEL="${1:-}"
INPUT_TOKENS="${2:-0}"
OUTPUT_TOKENS="${3:-0}"

if [[ -z "$MODEL" ]]; then
  echo "Usage: calculate-cost.sh <model> <input-tokens> <output-tokens>" >&2
  echo "" >&2
  echo "Models: sonnet-4.5, opus-4.5, haiku-4.5" >&2
  exit 1
fi

# Calculate cost based on model
case "$MODEL" in
  sonnet-4.5|sonnet)
    # $3/MTok input, $15/MTok output
    COST=$(awk "BEGIN {printf \"%.4f\", ($INPUT_TOKENS * 3.0 / 1000000) + ($OUTPUT_TOKENS * 15.0 / 1000000)}")
    MODEL_NAME="Claude Sonnet 4.5"
    ;;
  opus-4.5|opus)
    # $5/MTok input, $25/MTok output
    COST=$(awk "BEGIN {printf \"%.4f\", ($INPUT_TOKENS * 5.0 / 1000000) + ($OUTPUT_TOKENS * 25.0 / 1000000)}")
    MODEL_NAME="Claude Opus 4.5"
    ;;
  haiku-4.5|haiku)
    # $1/MTok input, $5/MTok output
    COST=$(awk "BEGIN {printf \"%.4f\", ($INPUT_TOKENS * 1.0 / 1000000) + ($OUTPUT_TOKENS * 5.0 / 1000000)}")
    MODEL_NAME="Claude Haiku 4.5"
    ;;
  *)
    echo "Unknown model: $MODEL" >&2
    echo "Supported models: sonnet-4.5, opus-4.5, haiku-4.5" >&2
    exit 1
    ;;
esac

# Display results
echo "Model: $MODEL_NAME"
echo "Input tokens: $INPUT_TOKENS"
echo "Output tokens: $OUTPUT_TOKENS"
echo "Total tokens: $((INPUT_TOKENS + OUTPUT_TOKENS))"
echo "Cost: \$$COST"
