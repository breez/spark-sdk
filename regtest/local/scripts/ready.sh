#!/bin/sh
# Says what the environment is still doing, then what it runs. This log is what
# starting the environment shows.
set -eu

. "$(dirname "$0")/lib.sh"

: "${LOCAL_DIR:?}" "${DKG_MIN_AVAILABLE_KEYS:?}" "${READY_TIMEOUT_SECONDS:=2400}"
: "${POSTGRES_HOST:?}" "${POSTGRES_PORT:?}" "${POSTGRES_USER:?}" "${POSTGRES_PASSWORD:?}"

# The fewest keyshares any operator coordinates, which is what the SSP waits on.
keyshares() {
  fewest=""
  for index in 0 1 2; do
    count=$(PGPASSWORD="$POSTGRES_PASSWORD" psql -h "$POSTGRES_HOST" -p "$POSTGRES_PORT" \
      -U "$POSTGRES_USER" -d "sparkoperator_$index" -tA \
      -c "SELECT count(*) FROM signing_keyshares WHERE status = 'AVAILABLE' AND coordinator_index = $index") || return 1
    if [ -z "$fewest" ] || [ "$count" -lt "$fewest" ]; then
      fewest=$count
    fi
  done
  echo "$fewest"
}

# Sets `message` to what the environment is waiting on. Not a subshell, so what
# it knows of earlier calls carries over.
generated=0
keys_ready=0
progress() {
  if [ "$keys_ready" -eq 0 ]; then
    if ! keys=$(keyshares 2>/dev/null); then
      message="the operators are starting"
      return
    fi
    if [ "$keys" -lt "$DKG_MIN_AVAILABLE_KEYS" ]; then
      # Kept at its high-water mark: a key taken while the rest are generated
      # would otherwise read as progress backwards.
      if [ "$keys" -gt "$generated" ]; then
        generated=$keys
      fi
      message="the operators are generating signing keys: $generated of $DKG_MIN_AVAILABLE_KEYS"
      return
    fi
    # Building the pool spends keys, and the operators generate more. Reporting
    # that as a return to the first phase would read as progress lost.
    keys_ready=1
  fi
  if ! stocked=$(pool_stocked 2>/dev/null); then
    message="the SSP is starting"
    return
  fi
  if [ "$stocked" -lt "$(pool_denominations)" ]; then
    message="the SSP is stocking its leaf pool: $stocked of $(pool_denominations) denominations"
    return
  fi
  if [ ! -f "$LOCAL_DIR/lightning-ready" ]; then
    message="Alice and the SSP's Lightning node are opening their channel"
    return
  fi
  if ! lnurl_serving; then
    message="the lnurl server is starting"
    return
  fi
  if ! data_sync_serving; then
    message="the data-sync service is starting"
    return
  fi
  message="the SSP is finishing its pool"
}

deadline=$(($(date +%s) + READY_TIMEOUT_SECONDS))
reported=""
while [ ! -f "$LOCAL_DIR/ready" ] || [ ! -f "$LOCAL_DIR/lightning-ready" ] ||
  ! lnurl_serving || ! data_sync_serving; do
  progress
  if [ "$(date +%s)" -ge "$deadline" ]; then
    log "still not ready after $((READY_TIMEOUT_SECONDS / 60)) minutes: $message"
    exit 1
  fi
  if [ "$message" != "$reported" ]; then
    log "$message"
    reported=$message
  fi
  sleep 5
done

# The chain API the SDK reads still lags the chain by a block or two here.
wait_for_chain_api

exec "$(dirname "$0")/summary.sh"
