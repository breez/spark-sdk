#!/bin/sh
# Where the environment's services are, and how to reach them.
set -eu

: "${SPARK_CONFIG_PATH:?}" "${BITCOIN_CLI:?}" "${SSP_CLI:?}"
: "${SSP_NODE_CLI:?}" "${ALICE_CLI:?}"
: "${BITCOIND_RPC_USER:?}" "${BITCOIND_RPC_PASSWORD:?}"
: "${PUBLIC_HOST:=127.0.0.1}" "${OPERATOR_PUBLIC_PORTS:=8535 8536 8537}"
: "${MEMPOOL_PORT:=8090}" "${SSP_PORT:=59049}" "${LNURL_PORT:=8080}"
: "${DATA_SYNC_PORT:=8081}"
: "${LDK_P2P_PORT:=9735}" "${LDK_ALICE_P2P_PORT:=9736}"
: "${BITCOIND_RPC_PORT:=18443}" "${BITCOIND_P2P_PORT:=18444}"

operators=""
for port in $OPERATOR_PUBLIC_PORTS; do
  operators="${operators:+$operators }https://$PUBLIC_HOST:$port"
done

cat <<EOF

Spark regtest environment ready

config
  $SPARK_CONFIG_PATH

services
  chain api  http://$PUBLIC_HOST:$MEMPOOL_PORT/api
  explorer   http://$PUBLIC_HOST:$MEMPOOL_PORT
  ssp        http://$PUBLIC_HOST:$SSP_PORT
  lnurl      http://$PUBLIC_HOST:$LNURL_PORT
  data sync  http://$PUBLIC_HOST:$DATA_SYNC_PORT
  operators  $operators
  ssp node   $PUBLIC_HOST:$LDK_P2P_PORT
  alice      $PUBLIC_HOST:$LDK_ALICE_P2P_PORT
  bitcoind   $PUBLIC_HOST:$BITCOIND_RPC_PORT $BITCOIND_RPC_USER/$BITCOIND_RPC_PASSWORD, p2p $PUBLIC_HOST:$BITCOIND_P2P_PORT

commands
  $BITCOIN_CLI getblockchaininfo
  $SSP_CLI pool status
  $SSP_NODE_CLI list-channels
  $ALICE_CLI bolt11-receive 10000sat
  $ALICE_CLI bolt11-send <invoice>

EOF
