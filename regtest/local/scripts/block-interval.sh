#!/bin/sh
# block-interval.sh <seconds>: how often the running miner mines a block, until
# it restarts. 0 mines only on request once the environment is ready.
set -eu

. "$(dirname "$0")/lib.sh"

: "${LOCAL_DIR:?}"

case "${1:-}" in
  '' | *[!0-9]*)
    echo "usage: block-interval.sh <seconds>" >&2
    exit 1
    ;;
esac

# Renamed into place, so the miner never reads a half-written file.
printf '%s\n' "$1" >"$LOCAL_DIR/block-interval.tmp"
mv "$LOCAL_DIR/block-interval.tmp" "$LOCAL_DIR/block-interval"
if [ "$1" -eq 0 ]; then
  log "mining only on request once the environment is ready"
else
  log "mining a block every ${1}s"
fi
