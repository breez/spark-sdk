#!/bin/sh
# Keeps the SSP's on-chain wallet funded, since it pays for the leaves it stocks
# its pool with. Writes LOCAL_DIR/ready once the pool holds
# LEAVES_PER_DENOMINATION leaves of every denomination up to
# 2^MAX_DENOMINATION_POWER sats.
set -eu

. "$(dirname "$0")/lib.sh"

: "${LOCAL_DIR:?}" "${LEAVES_PER_DENOMINATION:=8}" "${MAX_DENOMINATION_POWER:=16}"

# Twice what a full pool costs: the SSP also fronts coop exit withdrawals.
TARGET_SATS=24000000
UTXO_SATS=1000000

onchain_sats() {
  ssp_cli wallet balance | cut -d' ' -f1
}

top_up() {
  balance=$(onchain_sats)
  [ "$balance" -ge $((TARGET_SATS / 2)) ] && return 0
  count=$(((TARGET_SATS - balance) / UTXO_SATS))
  log "sending the SSP $count x $UTXO_SATS sats"
  i=0
  while [ "$i" -lt "$count" ]; do
    address=$(ssp_cli wallet new-address)
    rpc sendtoaddress "[\"$address\", 0.01]" >/dev/null
    i=$((i + 1))
  done
}

rm -f "$LOCAL_DIR/ready"
wait_for_bitcoind
# The miner creates the wallet and matures its coinbases.
until [ "$(rpc getbalance 2>/dev/null | jq -r 'floor')" -ge 1000 ] 2>/dev/null; do
  sleep 1
done
wait_for_ssp
top_up

log "waiting for the SSP to stock its pool"
while [ "$(pool_stocked)" -lt "$(pool_denominations)" ]; do
  sleep 5
done
touch "$LOCAL_DIR/ready"
log "ready"

while :; do
  sleep 60
  top_up || log "topping up the SSP failed"
done
