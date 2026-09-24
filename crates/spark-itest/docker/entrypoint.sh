#!/bin/sh
set -e


# Wait for postgres to be ready
until PGPASSWORD="$POSTGRES_PASSWORD" psql -h "$POSTGRES_HOST" -U "$POSTGRES_USER" -c '\q'; do
  echo "Postgres is unavailable - sleeping"
  sleep 1
done

echo "Postgres is up - preparing configuration"

# Default DB name if not specified
DB_NAME=${DB_NAME:-sparkoperator_${SPARK_OPERATOR_INDEX}}

# Update configuration with environment variables
CONFIG_FILE="/config/so.config.yaml"

# Check if the config file exists
if [ ! -f "$CONFIG_FILE" ]; then
  echo "Config file $CONFIG_FILE not found. Please mount it to the container."
  exit 1
fi

# Edits a copy, so a container that restarts starts again from the image's config.
RUN_CONFIG_FILE="/data/so.config.yaml"
cp "$CONFIG_FILE" "$RUN_CONFIG_FILE"

# Update bitcoind host if provided
if [ ! -z "$BITCOIND_HOST" ]; then
  echo "Updating bitcoind host to $BITCOIND_HOST"
  sed -i "s|host: 127.0.0.1:8332|host: $BITCOIND_HOST|g" "$RUN_CONFIG_FILE"
fi

# Update bitcoind zmqpubrawblock if provided
if [ ! -z "$BITCOIND_ZMQPUBRAWBLOCK" ]; then
  echo "Updating bitcoind zmqpubrawblock to $BITCOIND_ZMQPUBRAWBLOCK"
  sed -i "s|zmqpubrawblock: tcp://127.0.0.1:28332|zmqpubrawblock: $BITCOIND_ZMQPUBRAWBLOCK|g" "$RUN_CONFIG_FILE"
fi

# Update lrc20 host if provided
if [ ! -z "$LRC20_HOST" ]; then
  echo "Updating lrc20 host to $LRC20_HOST"
  sed -i "s|host: 127.0.0.1:18530|host: $LRC20_HOST|g" "$RUN_CONFIG_FILE"
fi

if [ ! -z "$DKG_MIN_AVAILABLE_KEYS" ]; then
  echo "Updating dkg min_available_keys to $DKG_MIN_AVAILABLE_KEYS"
  sed -i "s|min_available_keys: 100|min_available_keys: $DKG_MIN_AVAILABLE_KEYS|g" "$RUN_CONFIG_FILE"
fi

if [ ! -z "$DKG_BATCH_SIZE" ]; then
  echo "Updating dkg batch_size to $DKG_BATCH_SIZE"
  sed -i "s|spark.so.dkg.batch_size: 300|spark.so.dkg.batch_size: $DKG_BATCH_SIZE|g" "$RUN_CONFIG_FILE"
fi

rm -f "/data/key.txt"
echo $SPARK_OPERATOR_KEY > /data/key.txt

echo "Configuration updated, waiting for the operators.json file to be ready"

# Start the frost signer in the background
echo "Starting spark-frost-signer..."
spark-frost-signer -u /tmp/frost.sock 2>&1 | sed "s/^/[Signer] /" &
SIGNER_PID=$!

OPERATORS_JSON="${OPERATORS_JSON:-/config/operators.json}"
SERVER_CERT="${SERVER_CERT:-/data/server.crt}"
SERVER_KEY="${SERVER_KEY:-/data/server.key}"

# The file can exist before it lists the operators: their addresses may only be
# known once every operator container runs.
echo "Waiting for updated operators.json file..."
until grep -q identity_public_key "$OPERATORS_JSON" 2>/dev/null; do
  sleep 1
done
echo "operators.json lists the operators, proceeding with startup"

# Give the signer a moment to start up
sleep 1


echo "Starting spark operator..."
operator \
    -config "$RUN_CONFIG_FILE" \
    -index ${SPARK_OPERATOR_INDEX} \
    -key /data/key.txt \
    -server-cert "$SERVER_CERT" \
    -server-key "$SERVER_KEY" \
    -operators "$OPERATORS_JSON" \
    -threshold ${SPARK_THRESHOLD} \
    -signer "unix:///tmp/frost.sock" \
    -port 8535 \
    -database "postgresql://${POSTGRES_USER}:${POSTGRES_PASSWORD}@${POSTGRES_HOST}:${POSTGRES_PORT}/${DB_NAME}?sslmode=disable" \
    -run-dir "/data" \
    -local true 2>&1 | sed "s/^/[Operator] /" &
OPERATOR_PID=$!

# Monitor processes and exit if any of them fails
monitor_processes() {
  while true; do
    # Check if signer is still running
    if ! kill -0 $SIGNER_PID 2>/dev/null; then
      echo "Signer process died, shutting down container"
      [ -n "$OPERATOR_PID" ] && kill $OPERATOR_PID 2>/dev/null || true
      exit 1
    fi
    
    # Check if operator is still running
    if ! kill -0 $OPERATOR_PID 2>/dev/null; then
      echo "Operator process died, shutting down container"
      [ -n "$SIGNER_PID" ] && kill $SIGNER_PID 2>/dev/null || true
      exit 1
    fi
    
    sleep 5
  done
}

# Start the monitoring in background
monitor_processes &

# Wait for all background processes
wait
