#!/bin/sh
# Runs operator SPARK_OPERATOR_INDEX and its FROST signer, as the docker image's
# entrypoint does.
set -eu

: "${SPARK_LOCAL_DIR:?}" "${LOCAL_DIR:?}" "${SO_CONFIG:?}" "${SPARK_OPERATOR_INDEX:?}"
: "${SPARK_OPERATOR_KEY:?}" "${OPERATOR_PORT:?}" "${POSTGRES_PORT:?}"
: "${BITCOIND_RPC_PORT:?}" "${BITCOIND_ZMQ_PORT:?}"
: "${DKG_MIN_AVAILABLE_KEYS:?}" "${DKG_BATCH_SIZE:?}"

index=$SPARK_OPERATOR_INDEX
dir="$SPARK_LOCAL_DIR/operator-$index"
mkdir -p "$dir"

sed -e "s|host: 127.0.0.1:8332|host: 127.0.0.1:$BITCOIND_RPC_PORT|" \
  -e "s|zmqpubrawblock: tcp://127.0.0.1:28332|zmqpubrawblock: tcp://127.0.0.1:$BITCOIND_ZMQ_PORT|" \
  -e "s|min_available_keys: 100|min_available_keys: $DKG_MIN_AVAILABLE_KEYS|" \
  -e "s|spark.so.dkg.batch_size: 300|spark.so.dkg.batch_size: $DKG_BATCH_SIZE|" \
  "$SO_CONFIG" >"$dir/so.config.yaml"
printf '%s\n' "$SPARK_OPERATOR_KEY" >"$dir/key.txt"

# Each operator otherwise serves pprof on 127.0.0.1:6060.
export SPARK_PROFILING_PORT=$((16060 + index))

# A socket path is capped at around 100 bytes, which the data directory can exceed.
socket="/tmp/spark-local-$(id -u)-frost-$index.sock"
rm -f "$socket"
spark-frost-signer -u "$socket" &
signer=$!
trap 'kill "$signer" 2>/dev/null' EXIT INT TERM
until [ -S "$socket" ]; do
  sleep 0.2
done

operator \
  -config "$dir/so.config.yaml" \
  -index "$index" \
  -key "$dir/key.txt" \
  -server-cert "$LOCAL_DIR/certs/server.crt" \
  -server-key "$LOCAL_DIR/certs/server.key" \
  -operators "$LOCAL_DIR/operators.json" \
  -threshold 2 \
  -signer "unix://$socket" \
  -port "$OPERATOR_PORT" \
  -database "postgresql://postgres:postgres@127.0.0.1:$POSTGRES_PORT/sparkoperator_$index?sslmode=disable" \
  -run-dir "$dir" \
  -local true
