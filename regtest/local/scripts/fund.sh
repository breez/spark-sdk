#!/bin/sh
# fund.sh <address> <sats>: sends from the environment's bitcoind wallet.
set -eu

. "$(dirname "$0")/lib.sh"

: "${1:?address}" "${2:?sats}"
btc=$(printf '%d.%08d' $(($2 / 100000000)) $(($2 % 100000000)))
txid=$(rpc sendtoaddress "[\"$1\", $btc]" | jq -r .)
log "sent $2 sats to $1 in $txid"
