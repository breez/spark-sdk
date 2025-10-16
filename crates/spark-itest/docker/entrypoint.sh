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

# Make a copy of the original config file to work with
cp "$CONFIG_FILE" "$CONFIG_FILE.tmp"

# Update bitcoind host if provided
if [ ! -z "$BITCOIND_HOST" ]; then
  echo "Updating bitcoind host to $BITCOIND_HOST"
  sed -i "s|host: 127.0.0.1:8332|host: $BITCOIND_HOST|g" "$CONFIG_FILE.tmp"
fi

# Update bitcoind zmqpubrawblock if provided
if [ ! -z "$BITCOIND_ZMQPUBRAWBLOCK" ]; then
  echo "Updating bitcoind zmqpubrawblock to $BITCOIND_ZMQPUBRAWBLOCK"
  sed -i "s|zmqpubrawblock: tcp://127.0.0.1:28332|zmqpubrawblock: $BITCOIND_ZMQPUBRAWBLOCK|g" "$CONFIG_FILE.tmp"
fi

# Update lrc20 host if provided
if [ ! -z "$LRC20_HOST" ]; then
  echo "Updating lrc20 host to $LRC20_HOST"
  sed -i "s|host: 127.0.0.1:18530|host: $LRC20_HOST|g" "$CONFIG_FILE.tmp"
fi

# Replace the original config file with our modified version
mv "$CONFIG_FILE.tmp" "$CONFIG_FILE"

rm -f "/data/key.txt"
echo $SPARK_OPERATOR_KEY > /data/key.txt

echo "Configuration updated, waiting for the operators.json file to be ready"

# Start the frost signer in the background
echo "Starting spark-frost-signer..."
spark-frost-signer -u /tmp/frost.sock 2>&1 | sed "s/^/[Signer] /" &
SIGNER_PID=$!

# Create a timestamp file to track when operators.json was last modified
OPERATORS_JSON="/config/operators.json"
OPERATORS_TIMESTAMP_FILE="/tmp/operators_timestamp"

# Wait for operators.json to be created or updated
if [ -f "$OPERATORS_JSON" ]; then
  # Store initial timestamp if file exists
  stat -c %Y "$OPERATORS_JSON" > "$OPERATORS_TIMESTAMP_FILE"
else
  # File doesn't exist yet, set timestamp to 0
  echo "0" > "$OPERATORS_TIMESTAMP_FILE"
fi

echo "Waiting for updated operators.json file..."
while true; do
  if [ ! -f "$OPERATORS_JSON" ]; then
    echo "Waiting for operators.json to be created..."
    sleep 1
    continue
  fi
  
  CURRENT_TIMESTAMP=$(stat -c %Y "$OPERATORS_JSON")
  PREVIOUS_TIMESTAMP=$(cat "$OPERATORS_TIMESTAMP_FILE")
  
  if [ "$CURRENT_TIMESTAMP" -gt "$PREVIOUS_TIMESTAMP" ]; then
    echo "operators.json has been updated, proceeding with startup"
    break
  fi
  
  echo "Waiting for operators.json to be updated..."
  sleep 1
done

# Give the signer a moment to start up
sleep 1


echo "Starting spark operator..."
operator \
    -config "$CONFIG_FILE" \
    -index ${SPARK_OPERATOR_INDEX} \
    -key /data/key.txt \
    -server-cert "/data/server.crt" \
    -server-key "/data/server.key" \
    -operators "/config/operators.json" \
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
