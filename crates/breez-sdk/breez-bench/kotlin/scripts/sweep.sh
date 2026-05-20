#!/usr/bin/env bash
# RPS sweep driver: for each target RPS, spin up a fresh server,
# point loadgen at it for $DURATION, then tear it down. Each step
# gets its own out/<sweep-id>/rps-<N>/ directory so the aggregator
# can read them as a set.
#
# Per-step server restart isolates each RPS step from the previous
# one's resource state — TIME_WAIT sockets, leaked FDs, JVM heap drift
# all reset. Without it, a degraded step N would distort step N+1 and
# we'd misread carryover as RPS-driven cliff. Costs ~25s of JVM
# startup per step but keeps each step independently interpretable.
#
# Required env: MYSQL_URL, MASTER_SECRET.
# Optional env: SWEEP_RPS (default "50,100,250,500,1000"), DURATION
# (default 5m), MIX (info=40,receive=30,send=30),
# USERS (10000), SENDERS (50), DIST (uniform), PAYMENT_SATS (1),
# PORT (8080), SWEEP_ID (fresh timestamp),
# LOG_FILTER (unset = off; e.g. "warn,spark::operator::rpc=debug" turns
# on Rust SDK tracing in the per-step server → out/<id>/rps-N/.trace-logs/).

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
MIX="${MIX:-info=40,receive=30,send=30}"
USERS="${USERS:-10000}"
SENDERS="${SENDERS:-50}"
DIST="${DIST:-uniform}"
PAYMENT_SATS="${PAYMENT_SATS:-1}"
PORT="${PORT:-8080}"
# Output naming convention: <UTC-ISO-8601>[-<kebab-slug>]. Default is a
# fresh UTC timestamp; LABEL=<slug> appends a human-readable suffix so
# you can find the run later without remembering the timestamp. Set
# SWEEP_ID directly to override completely (e.g. resume / share a dir
# across phases). Enforced loosely below — non-conforming IDs warn but
# still run. See README "Output naming" for the canonical rule.
SWEEP_ID="${SWEEP_ID:-$(date -u +%Y-%m-%dT%H-%M-%SZ)${LABEL:+-${LABEL}}}"
SWEEP_ID_RE='^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}Z?(-[a-z0-9][a-z0-9-]*)?$'
if ! [[ "$SWEEP_ID" =~ $SWEEP_ID_RE ]]; then
    echo "[sweep] WARNING: SWEEP_ID='$SWEEP_ID' does not match the convention"
    echo "[sweep]          <YYYY-MM-DDTHH-MM-SSZ>[-<kebab-slug>] (see README 'Output naming')."
    echo "[sweep]          Continuing, but prefer LABEL=<slug> over a custom SWEEP_ID so out/ stays sortable."
fi
INTER_STEP_SLEEP_SECS="${INTER_STEP_SLEEP_SECS:-5}"
# Optional Rust-side SDK tracing in the per-step server. Unset = off (no
# overhead). A tracing EnvFilter string enables it; logs land in
# out/<id>/rps-<N>/.trace-logs/sdk.log. Scoped to the load-step server
# only (not fund/seed) to keep volume bounded.
LOG_FILTER="${LOG_FILTER:-}"
# Per-sender fund safety buffer (so we never drain a sender during the
# sweep) and treasurer buffer over total seeded amount.
SEED_SAFETY="${SEED_SAFETY:-2.0}"
TREASURER_BUFFER="${TREASURER_BUFFER:-1.5}"
# Floor: even info/receive-only sweeps need a small treasurer balance
# for the treasurer wallet to exist & for /receive to point somewhere.
TREASURER_MIN_FLOOR="${TREASURER_MIN_FLOOR:-50000}"

SWEEP_DIR="out/$SWEEP_ID"
mkdir -p "$SWEEP_DIR"

# Step duration in seconds for ETA computation. Match both upper and
# lower case suffixes since macOS ships bash 3.2 which has no native
# lowercase parameter expansion. Falls back to passing the raw value
# through if the suffix is unrecognised — ETA just won't be meaningful.
parse_duration_secs() {
    local s="$1"
    case "$s" in
        *h|*H) echo $(( ${s%[hH]} * 3600 )) ;;
        *m|*M) echo $(( ${s%[mM]} * 60 )) ;;
        *s|*S) echo "${s%[sS]}" ;;
        *)     echo "$s" ;;
    esac
}
STEP_DUR_S=$(parse_duration_secs "$DURATION" 2>/dev/null || echo 0)

# Run a long-lived gradle command with both file capture and live
# stdout filtering. Tees full output to $logfile, then surfaces lines
# matching $pattern (e.g. ^\[fund\]) to the controlling terminal.
# Stores the command's exit code in $STREAM_RC for the caller to read.
# Returns 0 always — set -e in the caller MUST NOT abort on a failed
# gradle command. Caller is responsible for handling STREAM_RC.
#
# (Returning $rc here would trip set -e at the call site BEFORE the
# caller can capture it, which silently kills the whole sweep on the
# first step where loadgen exits non-zero.)
stream_filtered() {
    local logfile="$1"
    local pattern="$2"
    shift 2
    set +e
    "$@" 2>&1 | tee "$logfile" | grep --line-buffered -E "$pattern"
    STREAM_RC=${PIPESTATUS[0]}
    set -e
    return 0
}

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
# Both are no-ops if treasurer/senders already at-or-above target. The
# stream_filtered helper writes full output to <logfile> while surfacing
# the bench's own status lines (^\[fund\] / ^\[seed\]) live to the
# terminal — including the new every-10s "still waiting" lines from
# waitForBalanceIncrease, so a slow faucet is visible.
echo
echo "[sweep] === fund treasurer (target=$TREASURER_TARGET) ==="
fund_log="$SWEEP_DIR/fund.log"
stream_filtered "$fund_log" '^\[fund\]' \
    ./gradlew run --console=plain --args="--mode=fund \
    --mysql-url=$MYSQL_URL \
    --master-secret=$MASTER_SECRET \
    --target-sats=$TREASURER_TARGET"
if [ "$STREAM_RC" -ne 0 ]; then
    echo "[sweep] fund failed (rc=$STREAM_RC); see $fund_log"
    exit "$STREAM_RC"
fi

if [ "$PER_SENDER_SATS" -gt 0 ]; then
    echo
    echo "[sweep] === seed senders (per-sender=$PER_SENDER_SATS, K=$SENDERS) ==="
    seed_log="$SWEEP_DIR/seed.log"
    stream_filtered "$seed_log" '^\[seed\]' \
        ./gradlew run --console=plain --args="--mode=seed-senders \
        --mysql-url=$MYSQL_URL \
        --master-secret=$MASTER_SECRET \
        --senders=$SENDERS \
        --per-sender-sats=$PER_SENDER_SATS"
    if [ "$STREAM_RC" -ne 0 ]; then
        echo "[sweep] seed-senders failed (rc=$STREAM_RC); see $seed_log"
        exit "$STREAM_RC"
    fi
else
    echo "[sweep] no /send in mix — skipping seed-senders"
fi

# --- treasurer address cache ---------------------------------------------
# The treasurer's Spark address is deterministic from the seed, but the
# only way to read it without rebuilding the SDK is via the server's
# /receive endpoint — and a fresh per-step server build pays the full
# `ensureSynced=true` cost for the treasurer wallet, which scales with
# its on-chain history (multi-minute on a wallet with several deposits).
#
# So we fetch it ONCE per master secret, cache to disk, and pass it
# through to every loadgen invocation. Cache file is keyed by a hash of
# MASTER_SECRET so swapping wallets invalidates automatically.
secret_hash() {
    printf '%s' "$MASTER_SECRET" | shasum -a 256 | cut -c1-16
}
TREASURER_ADDR_CACHE="$ROOT_DIR/out/.cache/treasurer-spark-addr.$(secret_hash).txt"
mkdir -p "$(dirname "$TREASURER_ADDR_CACHE")"

ensure_treasurer_addr() {
    if [ -s "$TREASURER_ADDR_CACHE" ]; then
        TREASURER_ADDR=$(cat "$TREASURER_ADDR_CACHE")
        echo "[sweep] using cached treasurer Spark addr: $TREASURER_ADDR"
        return 0
    fi
    echo "[sweep] no cached treasurer addr — fetching once (this can take several minutes the first time per master secret)"
    local bootstrap_log="$SWEEP_DIR/treasurer-bootstrap.log"
    ./gradlew run --console=plain --args="\
        --mode=server \
        --mysql-url=$MYSQL_URL \
        --master-secret=$MASTER_SECRET \
        --port=$PORT \
        --out-dir=$SWEEP_DIR/.bootstrap" \
        > "$bootstrap_log" 2>&1 &
    local bootstrap_pid=$!
    trap 'echo "[sweep] interrupted during treasurer bootstrap"; stop_server "$bootstrap_pid"; exit 130' INT TERM
    if ! wait_for_server 120; then
        echo "[sweep] treasurer-bootstrap server did not become ready (see $bootstrap_log)"
        stop_server "$bootstrap_pid"
        trap - INT TERM
        return 1
    fi
    # No HTTP timeout — the treasurer cold-start is whatever it is.
    local addr
    addr=$(curl -sS --max-time 0 -X POST -H 'content-type: application/json' \
        -d '{}' "http://localhost:$PORT/users/__treasurer__/receive" \
        | python3 -c 'import json,sys; print(json.load(sys.stdin)["paymentRequest"])')
    stop_server "$bootstrap_pid"
    trap - INT TERM
    if [ -z "$addr" ]; then
        echo "[sweep] failed to fetch treasurer Spark addr (see $bootstrap_log)"
        return 1
    fi
    printf '%s\n' "$addr" > "$TREASURER_ADDR_CACHE"
    TREASURER_ADDR="$addr"
    echo "[sweep] cached treasurer Spark addr → $TREASURER_ADDR_CACHE"
    echo "[sweep] treasurer destination: $TREASURER_ADDR"
}

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

# Stop the server cleanly.
#
# `./gradlew run &` launches the gradle wrapper, which forks a separate
# JVM for the application. SIGTERM-ing the gradle wrapper does NOT
# reliably propagate to the JVM child — on macOS it commonly orphans,
# leaving the JVM listening on $PORT after the script "stopped" the
# server. To make this robust we (a) SIGTERM gradle, (b) actively look
# for any process still holding $PORT and SIGTERM it (giving the JVM's
# shutdown hook a chance to flush requests.jsonl + metrics.jsonl), then
# (c) escalate to SIGKILL if anything is still alive after a grace period.
stop_server() {
    local pid="$1"
    if kill -0 "$pid" 2>/dev/null; then
        kill -TERM "$pid" 2>/dev/null || true
    fi
    # Find any JVM still bound to $PORT (the application JVM that's a
    # child of gradle, plus anything else that happens to be there).
    local lingering
    lingering=$(lsof -nP -iTCP:"$PORT" -sTCP:LISTEN -t 2>/dev/null || true)
    if [ -n "$lingering" ]; then
        kill -TERM $lingering 2>/dev/null || true
    fi
    # Wait up to 15s for everything to release the port. The shutdown
    # hook drains the JSONL writers (per-write flush + 10s join), so we
    # need at least that much before escalating.
    local deadline=$(( $(date +%s) + 15 ))
    while [ "$(date +%s)" -lt "$deadline" ]; do
        local still
        still=$(lsof -nP -iTCP:"$PORT" -sTCP:LISTEN -t 2>/dev/null || true)
        if [ -z "$still" ] && ! kill -0 "$pid" 2>/dev/null; then
            return 0
        fi
        sleep 1
    done
    echo "[sweep] server didn't release :$PORT in 15s — sending SIGKILL"
    kill -KILL "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    local still
    still=$(lsof -nP -iTCP:"$PORT" -sTCP:LISTEN -t 2>/dev/null || true)
    if [ -n "$still" ]; then
        kill -KILL $still 2>/dev/null || true
    fi
}

# Fetch (or load cached) treasurer Spark address now that helpers are
# defined. Done after fund + seed so the wallet is guaranteed to exist.
ensure_treasurer_addr || exit 1

# --- main loop ------------------------------------------------------------

IFS=',' read -ra RPS_LIST <<< "$SWEEP_RPS"
STEP_COUNT=${#RPS_LIST[@]}
i=0

for raw_rps in "${RPS_LIST[@]}"; do
    i=$((i + 1))
    rps="$(echo "$raw_rps" | tr -d ' ')"
    step_dir="$SWEEP_DIR/rps-$rps"
    mkdir -p "$step_dir"

    # ETA: per-step duration plus the inter-step drain, times the
    # number of steps remaining (including this one).
    remaining_steps=$((STEP_COUNT - i + 1))
    eta_s=$(( remaining_steps * STEP_DUR_S + (remaining_steps - 1) * INTER_STEP_SLEEP_SECS ))
    eta_min=$(( eta_s / 60 ))
    step_start_ts=$(date +%s)
    echo
    echo "[sweep] === step $i/$STEP_COUNT  rps=$rps  out=$step_dir  (~${eta_min}m sweep remaining) ==="

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
        --out-dir=$step_dir \
        ${LOG_FILTER:+--log-filter=$LOG_FILTER}" \
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

    # Loadgen produces a [loadgen] +Xs progress line every 5s; surface
    # those live so the user sees dispatched / in_flight / errors as
    # they grow rather than waiting for $DURATION of silence.
    stream_filtered "$step_dir/loadgen.log" '^\[loadgen\]' \
        ./gradlew run --console=plain --args="\
        --mode=loadgen \
        --base-url=http://localhost:$PORT \
        --target-rps=$rps \
        --duration=$DURATION \
        --users=$USERS \
        --mix=$MIX \
        --user-distribution=$DIST \
        --senders=$SENDERS \
        --payment-sats=$PAYMENT_SATS \
        --run-id=$SWEEP_ID/rps-$rps \
        --out-dir=$step_dir \
        --treasurer-spark-addr=$TREASURER_ADDR"
    loadgen_rc=$STREAM_RC

    stop_server "$server_pid"
    trap - INT TERM

    step_elapsed=$(( $(date +%s) - step_start_ts ))
    if [ "$loadgen_rc" -ne 0 ]; then
        # Don't abort the sweep — the aggregator will skip steps with
        # missing/empty data, and the next RPS step might be cleaner.
        echo "[sweep] step $i/$STEP_COUNT FAILED in ${step_elapsed}s (loadgen rc=$loadgen_rc; see $step_dir/loadgen.log) — continuing"
    else
        echo "[sweep] step $i/$STEP_COUNT done in ${step_elapsed}s"
    fi

    if [ "$loadgen_rc" -ne 0 ]; then
        echo "[sweep] loadgen exited rc=$loadgen_rc — see $step_dir/loadgen.log"
    fi

    # Stop the sweep once a step congestion-collapses. Higher-RPS steps
    # past the collapse knee are not reproducible measurements (external
    # operator rate limit + closed-loop funding feedback + per-step
    # SIGKILL truncation), so running them only burns time and emits junk
    # rows. We keep THIS step's data (it's the one collapsed point that
    # documents the cliff) and stop before the next.
    step_st=$(python3 "$SCRIPT_DIR/aggregate.py" --step-state "$step_dir" 2>/dev/null || echo collapsed)
    echo "[sweep] step $i/$STEP_COUNT state: $step_st"
    if [ "$step_st" = "collapsed" ]; then
        echo "[sweep] congestion collapse at rps=$rps — skipping remaining steps (post-collapse data is not reproducible)"
        break
    fi

    if [ "$i" -lt "$STEP_COUNT" ]; then
        echo "[sweep] sleeping ${INTER_STEP_SLEEP_SECS}s before next step (let TIME_WAIT sockets drain)"
        sleep "$INTER_STEP_SLEEP_SECS"
    fi
done

echo
echo "[sweep] done. Steps in $SWEEP_DIR/. Aggregate with: make aggregate SWEEP_ID=$SWEEP_ID"
