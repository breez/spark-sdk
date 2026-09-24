#!/bin/sh
# Writes the mempool backend's config from MEMPOOL_TEMPLATE, with the ports this
# environment runs on. The ports are quoted in the template and unquoted here,
# which JSON needs.
set -eu

: "${MEMPOOL_TEMPLATE:?}" "${SPARK_LOCAL_DIR:?}" "${MEMPOOL_API_PORT:?}" "${ELECTRS_PORT:?}"
: "${BITCOIND_RPC_PORT:?}" "${BITCOIND_RPC_USER:?}" "${BITCOIND_RPC_PASSWORD:?}"

dir="$SPARK_LOCAL_DIR/mempool"
mkdir -p "$dir/cache"

sed -e "s|@CACHE_DIR@|$dir/cache|" \
  -e "s|\"@MEMPOOL_API_PORT@\"|$MEMPOOL_API_PORT|" \
  -e "s|\"@BITCOIND_RPC_PORT@\"|$BITCOIND_RPC_PORT|" \
  -e "s|@ELECTRS_PORT@|$ELECTRS_PORT|" \
  -e "s|@BITCOIND_RPC_USER@|$BITCOIND_RPC_USER|" \
  -e "s|@BITCOIND_RPC_PASSWORD@|$BITCOIND_RPC_PASSWORD|" \
  "$MEMPOOL_TEMPLATE" >"$dir/mempool-config.json"
