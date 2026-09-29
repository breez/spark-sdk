#!/bin/sh
# Mines a block every BLOCK_INTERVAL_SECONDS, so deposits and the SSP's
# transactions confirm without anyone mining by hand. block-interval.sh changes
# the interval until the miner restarts. At 0 it mines until the environment is
# ready, and after that only on request.
set -eu

. "$(dirname "$0")/lib.sh"

: "${LOCAL_DIR:?}" "${BLOCK_INTERVAL_SECONDS:=5}"

# Pays out of the wallet after this height, so it does not track a coinbase for
# every block of a chain that runs for days.
WALLET_MINING_HEIGHT=200

# The SSP's pool and Alice's channel confirm on blocks, so an interval of 0
# still mines this often until they are ready.
SETUP_INTERVAL_SECONDS=5

wait_for_bitcoind
ensure_wallet
wallet_address=$(rpc_string getnewaddress '["mining", "bech32"]')

mine() {
  if [ "$(rpc getblockcount)" -lt "$WALLET_MINING_HEIGHT" ]; then
    address=$wallet_address
  else
    address=$UNOWNED_ADDRESS
  fi
  rpc generatetoaddress "[$1, \"$address\"]" >/dev/null
}

environment_ready() {
  [ -f "$LOCAL_DIR/ready" ] && [ -f "$LOCAL_DIR/lightning-ready" ]
}

height=$(rpc getblockcount)
if [ "$height" -lt "$WALLET_MINING_HEIGHT" ]; then
  log "mining to height $WALLET_MINING_HEIGHT"
  mine $((WALLET_MINING_HEIGHT - height))
fi

rm -f "$LOCAL_DIR/block-interval"

# Checks the interval every second, so a new one takes effect at once.
reported=""
elapsed=0
while :; do
  seconds=$(cat "$LOCAL_DIR/block-interval" 2>/dev/null || echo "$BLOCK_INTERVAL_SECONDS")
  if [ "$seconds" -gt 0 ]; then
    pace="a block every ${seconds}s"
  elif environment_ready; then
    pace="only on request"
  else
    seconds=$SETUP_INTERVAL_SECONDS
    pace="a block every ${seconds}s until the environment is ready"
  fi
  if [ "$pace" != "$reported" ]; then
    log "mining $pace"
    reported=$pace
    elapsed=0
  fi

  sleep 1
  elapsed=$((elapsed + 1))
  if [ "$seconds" -gt 0 ] && [ "$elapsed" -ge "$seconds" ]; then
    mine 1 || log "mining a block failed"
    elapsed=0
  fi
done
