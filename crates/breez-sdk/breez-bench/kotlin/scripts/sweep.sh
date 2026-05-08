#!/usr/bin/env bash
# RPS sweep driver: for each target RPS, spin up a fresh server,
# point loadgen at it for $DURATION, then tear it down. Each step
# gets its own out/<sweep-id>/rps-<N>/ directory so the aggregator
# can read them as a set.
#
# Per-step server restart matters: cold-start latency is part of the
# v1 baseline. Reusing a single server across steps would amortise
# that and overstate v1 capacity.
#
# Required env: MYSQL_URL, MASTER_SECRET.
# Optional env: SWEEP_RPS (default "50,100,250,500,1000"), DURATION
# (default 5m), WARMUP_SECS (60), MIX (info=40,receive=30,send=30),
# USERS (10000), SENDERS (50), DIST (uniform), PAYMENT_SATS (1),
# PORT (8080), SWEEP_ID (fresh timestamp).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

: "${MYSQL_URL:?MYSQL_URL is required (e.g. mysql://root:password@127.0.0.1:3306/breez_bench)}"
# MASTER_SECRET is defaulted by the Makefile; honour what's set so the
# script also works when invoked outside `make`.
MASTER_SECRET="${MASTER_SECRET:-breez-bench}"

SWEEP_RPS="${SWEEP_RPS:-50,100,250,500,1000}"
DURATION="${DURATION:-5m}"
WARMUP_SECS="${WARMUP_SECS:-60}"
MIX="${MIX:-info=40,receive=30,send=30}"
USERS="${USERS:-10000}"
SENDERS="${SENDERS:-50}"
DIST="${DIST:-uniform}"
PAYMENT_SATS="${PAYMENT_SATS:-1}"
PORT="${PORT:-8080}"
SWEEP_ID="${SWEEP_ID:-$(date +%Y-%m-%dT%H-%M-%S)}"
INTER_STEP_SLEEP_SECS="${INTER_STEP_SLEEP_SECS:-5}"
# Per-sender fund safety buffer (so we never drain a sender during the
# sweep) and treasurer buffer over total seeded amount.
SEED_SAFETY="${SEED_SAFETY:-2.0}"
TREASURER_BUFFER="${TREASURER_BUFFER:-1.5}"
# Floor: even info/receive-only sweeps need a small treasurer balance
# for the treasurer wallet to exist & for /receive to point somewhere.
TREASURER_MIN_FLOOR="${TREASURER_MIN_FLOOR:-50000}"

SWEEP_DIR="out/$SWEEP_ID"
mkdir -p "$SWEEP_DIR"

echo "[sweep] sweep-id=$SWEEP_ID  rps=[$SWEEP_RPS]  duration=$DURATION  out=$SWEEP_DIR"

# --- workload-sized funding ----------------------------------------------
# Compute per_sender_sats and treasurer_target from the sweep config so
# we only ever request the faucet sats this sweep actually needs (and
# only on the first run — subsequent runs are no-ops since the closed
# loop preserves the system total).

read PER_SENDER_SATS TREASURER_TARGET < <(
    python3 "$SCRIPT_DIR/compute_funding.py" \
        --rps "$SWEEP_RPS" \
        --duration "$DURATION" \
        --mix "$MIX" \
        --senders "$SENDERS" \
        --payment-sats "$PAYMENT_SATS" \
        --safety "$SEED_SAFETY" \
        --buffer "$TREASURER_BUFFER" \
        --floor "$TREASURER_MIN_FLOOR"
)

echo "[sweep] computed funding budget:"
echo "[sweep]   per-sender = ${PER_SENDER_SATS} sats  (sends-per-sender × payment × safety=${SEED_SAFETY})"
echo "[sweep]   treasurer  = ${TREASURER_TARGET} sats (K × per-sender × buffer=${TREASURER_BUFFER}, floor=${TREASURER_MIN_FLOOR})"

# Manifest captures the inputs so the aggregator (and a partner reader)
# can reconstruct what the sweep was. Written before any steps run so
# even an aborted sweep leaves provenance behind.
cat > "$SWEEP_DIR/manifest.json" <<EOF
{
  "sweep_id": "$SWEEP_ID",
  "rps_steps": [$(echo "$SWEEP_RPS" | sed 's/,/, /g')],
  "duration_per_step": "$DURATION",
  "warmup_secs": $WARMUP_SECS,
  "mix": "$MIX",
  "users": $USERS,
  "senders": $SENDERS,
  "distribution": "$DIST",
  "payment_sats": $PAYMENT_SATS,
  "port": $PORT,
  "funding": {
    "per_sender_sats": $PER_SENDER_SATS,
    "treasurer_target_sats": $TREASURER_TARGET,
    "seed_safety": $SEED_SAFETY,
    "treasurer_buffer": $TREASURER_BUFFER
  },
  "started_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "host": "$(uname -srm)"
}
EOF

# --- pre-sweep fund + seed (idempotent) -----------------------------------
# Both are no-ops if treasurer/senders already at-or-above target. Output
# trimmed to the bench's own status lines; full Gradle output goes to log.
echo
echo "[sweep] === fund treasurer (target=$TREASURER_TARGET) ==="
fund_log="$SWEEP_DIR/fund.log"
./gradlew run --console=plain --args="--mode=fund \
    --mysql-url=$MYSQL_URL \
    --master-secret=$MASTER_SECRET \
    --target-sats=$TREASURER_TARGET" \
    > "$fund_log" 2>&1
fund_rc=$?
grep -E '^\[fund\]' "$fund_log" || true
if [ "$fund_rc" -ne 0 ]; then
    echo "[sweep] fund failed (rc=$fund_rc); see $fund_log"
    exit "$fund_rc"
fi

if [ "$PER_SENDER_SATS" -gt 0 ]; then
    echo
    echo "[sweep] === seed senders (per-sender=$PER_SENDER_SATS, K=$SENDERS) ==="
    seed_log="$SWEEP_DIR/seed.log"
    ./gradlew run --console=plain --args="--mode=seed-senders \
        --mysql-url=$MYSQL_URL \
        --master-secret=$MASTER_SECRET \
        --senders=$SENDERS \
        --per-sender-sats=$PER_SENDER_SATS" \
        > "$seed_log" 2>&1
    seed_rc=$?
    grep -E '^\[seed\]' "$seed_log" | tail -10 || true
    if [ "$seed_rc" -ne 0 ]; then
        echo "[sweep] seed-senders failed (rc=$seed_rc); see $seed_log"
        exit "$seed_rc"
    fi
else
    echo "[sweep] no /send in mix — skipping seed-senders"
fi

# --- helpers --------------------------------------------------------------

wait_for_server() {
    local timeout_s="${1:-90}"
    local deadline=$(( $(date +%s) + timeout_s ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if curl -sf "http://localhost:$PORT/users/__healthcheck__/info" >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    return 1
}

# Stop the server cleanly. SIGTERM lets the shutdown hook flush
# metrics.jsonl + requests.jsonl; we only escalate to SIGKILL if it
# refuses to die.
stop_server() {
    local pid="$1"
    if ! kill -0 "$pid" 2>/dev/null; then
        return 0
    fi
    kill -TERM "$pid" 2>/dev/null || true
    local deadline=$(( $(date +%s) + 15 ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        if ! kill -0 "$pid" 2>/dev/null; then
            return 0
        fi
        sleep 1
    done
    echo "[sweep] server pid=$pid did not exit in 15s — sending SIGKILL"
    kill -KILL "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
}

# --- main loop ------------------------------------------------------------

IFS=',' read -ra RPS_LIST <<< "$SWEEP_RPS"
STEP_COUNT=${#RPS_LIST[@]}
i=0

for raw_rps in "${RPS_LIST[@]}"; do
    i=$((i + 1))
    rps="$(echo "$raw_rps" | tr -d ' ')"
    step_dir="$SWEEP_DIR/rps-$rps"
    mkdir -p "$step_dir"

    echo
    echo "[sweep] === step $i/$STEP_COUNT  rps=$rps  out=$step_dir ==="

    # The Gradle wrapper writes a lot of its own boilerplate to stdout
    # which we don't want in the run log; --console=plain trims it but
    # logback still emits the bench's own [server] lines.
    server_log="$step_dir/server.log"
    ./gradlew run --console=plain --args="\
        --mode=server \
        --mysql-url=$MYSQL_URL \
        --master-secret=$MASTER_SECRET \
        --port=$PORT \
        --run-id=$SWEEP_ID/rps-$rps \
        --out-dir=$step_dir" \
        > "$server_log" 2>&1 &
    server_pid=$!
    # Killing gradlew alone can leak the JVM child; trap handles the SIGINT path.
    trap 'echo "[sweep] interrupted — stopping server"; stop_server "$server_pid"; exit 130' INT TERM

    if ! wait_for_server 120; then
        echo "[sweep] server did not become ready in 120s — aborting step (see $server_log)"
        stop_server "$server_pid"
        trap - INT TERM
        continue
    fi
    echo "[sweep] server ready"

    # Loadgen runs in the foreground; on success it produces latency.jsonl
    # in $step_dir alongside the server's requests.jsonl + metrics.jsonl.
    set +e
    ./gradlew run --console=plain --args="\
        --mode=loadgen \
        --base-url=http://localhost:$PORT \
        --target-rps=$rps \
        --duration=$DURATION \
        --users=$USERS \
        --mix=$MIX \
        --user-distribution=$DIST \
        --warmup-secs=$WARMUP_SECS \
        --senders=$SENDERS \
        --payment-sats=$PAYMENT_SATS \
        --run-id=$SWEEP_ID/rps-$rps \
        --out-dir=$step_dir" \
        > "$step_dir/loadgen.log" 2>&1
    loadgen_rc=$?
    set -e

    stop_server "$server_pid"
    trap - INT TERM

    if [ "$loadgen_rc" -ne 0 ]; then
        echo "[sweep] loadgen exited rc=$loadgen_rc — see $step_dir/loadgen.log"
    fi

    if [ "$i" -lt "$STEP_COUNT" ]; then
        echo "[sweep] sleeping ${INTER_STEP_SLEEP_SECS}s before next step (let TIME_WAIT sockets drain)"
        sleep "$INTER_STEP_SLEEP_SECS"
    fi
done

echo
echo "[sweep] done. Steps in $SWEEP_DIR/. Aggregate with: make aggregate SWEEP_ID=$SWEEP_ID"
