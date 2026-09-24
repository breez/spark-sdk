#!/bin/sh
# mine.sh <blocks>
set -eu

. "$(dirname "$0")/lib.sh"

: "${1:?blocks}"
rpc generatetoaddress "[$1, \"$UNOWNED_ADDRESS\"]" >/dev/null
log "mined $1 blocks, height $(rpc getblockcount)"
