#!/bin/sh
# Gives Alice a channel with the SSP's Lightning node, funded on both sides, so
# a Lightning payment can leave and enter the environment. Writes
# LOCAL_DIR/lightning-ready once the channel can carry one.
set -eu

. "$(dirname "$0")/lib.sh"

: "${LOCAL_DIR:?}" "${SSP_NODE_ADDRESS:?}" "${SSP_BASE_URL:?}" "${SSP_TLS_CERT:?}"
: "${LDK_SERVER_API_KEY:?}"
: "${ALICE_CONFIG:=$LOCAL_DIR/ldk-alice.toml}" "${CHANNEL_SATS:=5000000000}"
# Alice's config names the address she listens on, which is another host's here.
: "${ALICE_BASE_URL:=}"

# The SSP's node is asked for its own id, which follows from the keys it holds.
ssp() {
  ldk-server-cli --base-url "$SSP_BASE_URL" --api-key "$LDK_SERVER_API_KEY" \
    --tls-cert "$SSP_TLS_CERT" "$@"
}

alice() {
  if [ -n "$ALICE_BASE_URL" ]; then
    ldk-server-cli --config "$ALICE_CONFIG" --base-url "$ALICE_BASE_URL" "$@"
  else
    ldk-server-cli --config "$ALICE_CONFIG" "$@"
  fi
}

# The channel carries a payment in both directions once it is usable, since half
# of it was pushed to the SSP when it was opened.
channel_usable() {
  alice list-channels | jq -e --arg ssp "$ssp_node_id" '[.channels[]
    | select((.isUsable // .is_usable) and (.counterpartyNodeId // .counterparty_node_id) == $ssp)]
    | length > 0' >/dev/null
}

# Spendable, not total: a channel is funded from confirmed coins.
onchain_sats() {
  alice get-balances | jq -r '.spendableOnchainBalanceSats // .spendable_onchain_balance_sats'
}

rm -f "$LOCAL_DIR/lightning-ready"
wait_for_bitcoind
until alice get-node-info >/dev/null 2>&1 && ssp get-node-info >/dev/null 2>&1; do
  sleep 1
done
ssp_node_id=$(ssp get-node-info | jq -r '.nodeId // .node_id')

if ! channel_usable; then
  # The channel's own value, plus what an anchor channel keeps back on chain.
  funding_sats=$((CHANNEL_SATS + 100000000))
  if [ "$(onchain_sats)" -lt "$funding_sats" ]; then
    # The chain's coinbases mature before they can pay for a channel this size.
    until [ "$(rpc getbalance | jq -r 'floor')" -ge $((funding_sats / 100000000 + 1)) ]; do
      sleep 2
    done
    address=$(alice onchain-receive | jq -r '.address')
    log "funding Alice with $funding_sats sats"
    btc=$(printf '%d.%08d' $((funding_sats / 100000000)) $((funding_sats % 100000000)))
    rpc sendtoaddress "[\"$address\", $btc]" >/dev/null
    until [ "$(onchain_sats)" -ge "$funding_sats" ]; do
      sleep 2
    done
  fi

  log "opening a ${CHANNEL_SATS} sat channel between Alice and the SSP's node"
  alice open-channel "$ssp_node_id" "$SSP_NODE_ADDRESS" "${CHANNEL_SATS}sat" \
    --push-to-counterparty "$((CHANNEL_SATS / 2))sat" >/dev/null
fi

until channel_usable; do
  sleep 2
done

touch "$LOCAL_DIR/lightning-ready"
log "the channel carries payments both ways"
