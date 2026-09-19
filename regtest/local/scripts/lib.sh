# Sourced by the other scripts. Needs BITCOIND_RPC_URL, BITCOIND_RPC_USER,
# BITCOIND_RPC_PASSWORD and SSP_GRPC_URI.

# Mined to when no wallet should own the coinbase.
UNOWNED_ADDRESS=bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw

log() {
  echo "[$(basename "$0" .sh)] $*"
}

# rpc <method> [params json] [wallet]. Prints the result, fails on an RPC error.
rpc() {
  _path=""
  [ -n "${3:-}" ] && _path="/wallet/$3"
  _response=$(curl -s --user "$BITCOIND_RPC_USER:$BITCOIND_RPC_PASSWORD" \
    -H 'content-type: text/plain' \
    --data "{\"jsonrpc\":\"1.0\",\"id\":\"local\",\"method\":\"$1\",\"params\":${2:-[]}}" \
    "$BITCOIND_RPC_URL$_path") || return 1
  if [ "$(printf '%s\n' "$_response" | jq '.error')" != "null" ]; then
    printf '%s\n' "$_response" | jq -c '.error' >&2
    return 1
  fi
  printf '%s\n' "$_response" | jq -c '.result'
}

wait_for_bitcoind() {
  until rpc getblockchaininfo >/dev/null 2>&1; do
    sleep 1
  done
}

# The wallet funds the SSP and the faucet. It is loaded on startup once created.
ensure_wallet() {
  rpc listwallets | jq -e 'index("default")' >/dev/null && return 0
  rpc loadwallet '["default"]' >/dev/null 2>&1 && return 0
  rpc createwallet '["default", false, false, "", false, true, true]' >/dev/null
}

ssp_cli() {
  ssp-cli --grpc-uri "$SSP_GRPC_URI" "$@"
}

# How many denominations hold LEAVES_PER_DENOMINATION leaves or more.
pool_stocked() {
  ssp_cli pool status | awk -v min="$LEAVES_PER_DENOMINATION" -v power="$MAX_DENOMINATION_POWER" '
    /^requested/ { done = 1 }
    !done && / sats x / { count[$1] = $4 }
    END {
      stocked = 0
      for (p = 0; p <= power; p++) if (count[2 ^ p] + 0 >= min) stocked++
      print stocked
    }'
}

pool_denominations() {
  echo $((MAX_DENOMINATION_POWER + 1))
}

# The chain API answers from the block the SDK's wallets are about to read, not
# from a block electrs has yet to index.
wait_for_chain_api() {
  : "${CHAIN_API_URL:?}"
  until [ "$(curl -sf "$CHAIN_API_URL/blocks/tip/height" || echo -1)" -ge "$(rpc getblockcount)" ]; do
    sleep 1
  done
}

# Answers once the lnurl server serves the addresses wallets register with it.
lnurl_serving() {
  curl -sf -o /dev/null "${LNURL_URL:?}/health"
}

# Answers once the data-sync service serves the wallets syncing through it.
data_sync_serving() {
  curl -sf -o /dev/null "${DATA_SYNC_WEB_URL:?}/"
}

wait_for_ssp() {
  until ssp_cli get-info >/dev/null 2>&1; do
    sleep 1
  done
}
