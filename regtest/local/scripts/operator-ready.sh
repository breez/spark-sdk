#!/bin/sh
# Succeeds once the operator serves its API and coordinates
# DKG_MIN_AVAILABLE_KEYS unused keyshares. The SSP's first attempt at its pool
# reserves thousands of them, and a cycle that runs short keeps what it
# reserved.
set -eu

: "${OPERATOR_ADDRESS:=127.0.0.1:8535}"

openssl s_client -connect "$OPERATOR_ADDRESS" </dev/null >/dev/null 2>&1

count=$(PGPASSWORD="$POSTGRES_PASSWORD" psql -h "$POSTGRES_HOST" -p "$POSTGRES_PORT" \
  -U "$POSTGRES_USER" -d "${DB_NAME:-sparkoperator_$SPARK_OPERATOR_INDEX}" -tA \
  -c "SELECT count(*) FROM signing_keyshares WHERE status = 'AVAILABLE' AND coordinator_index = $SPARK_OPERATOR_INDEX")
[ "$count" -ge "$DKG_MIN_AVAILABLE_KEYS" ]
