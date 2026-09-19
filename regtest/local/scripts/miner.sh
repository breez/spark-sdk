#!/bin/sh
# Mines a block every BLOCK_INTERVAL_SECONDS, so deposits and the SSP's
# transactions confirm without anyone mining by hand. 0 mines nothing after the
# chain is set up.
set -eu

. "$(dirname "$0")/lib.sh"

: "${BLOCK_INTERVAL_SECONDS:=5}"

# Pays out of the wallet after this height, so it does not track a coinbase for
# every block of a chain that runs for days.
WALLET_MINING_HEIGHT=200

wait_for_bitcoind
ensure_wallet
wallet_address=$(rpc getnewaddress '["mining", "bech32"]' | jq -r .)

mine() {
  if [ "$(rpc getblockcount)" -lt "$WALLET_MINING_HEIGHT" ]; then
    address=$wallet_address
  else
    address=$UNOWNED_ADDRESS
  fi
  rpc generatetoaddress "[$1, \"$address\"]" >/dev/null
}

height=$(rpc getblockcount)
if [ "$height" -lt "$WALLET_MINING_HEIGHT" ]; then
  log "mining to height $WALLET_MINING_HEIGHT"
  mine $((WALLET_MINING_HEIGHT - height))
fi

if [ "$BLOCK_INTERVAL_SECONDS" -eq 0 ]; then
  log "chain ready, not mining further"
  exec sleep infinity
fi

log "mining a block every ${BLOCK_INTERVAL_SECONDS}s"
while :; do
  sleep "$BLOCK_INTERVAL_SECONDS"
  mine 1 || log "mining a block failed"
done
