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
# 120s default lets SSP / operator rate-limit windows reset between steps so
# step N+1 isn't sampling step N's tail-end congestion state. Cheap insurance
# against cross-step contamination; override to 5 for fast iteration if you
# know the steps don't push external limits.
INTER_STEP_SLEEP_SECS="${INTER_STEP_SLEEP_SECS:-120}"
# Optional Rust-side SDK tracing in the per-step server. Unset = off (no
# overhead). A tracing EnvFilter string enables it; logs land in
# out/<id>/rps-<N>/.trace-logs/sdk.log. Scoped to the load-step server
# only (not fund/seed) to keep volume bounded.
LOG_FILTER="${LOG_FILTER:-}"
# Bench-report tracing preset (leaves-swap detection + per-RPC close-event
# spans, consumed by aggregate.py to render the "Leaves swap / optimization"
# and "Slow payments — per-RPC breakdown" sections in RESULTS.md). The
# preset is ON by default in the server; set BENCH_TRACE=0 here to A/B
# the instrumentation overhead. If LOG_FILTER is also set, the explicit
# filter wins — user retains full control.
BENCH_TRACE="${BENCH_TRACE:-1}"
# Per-sender fund safety buffer (so we never drain a sender during the
# sweep) and treasurer buffer over total seeded amount.
SEED_SAFETY="${SEED_SAFETY:-2.0}"
# Pool-count safety for send_ln only; separate from SEED_SAFETY because pool
# overshoot is cheap (1-2 extra dispatches from dispatch-loop rounding) but
# sender-drain overshoot is fatal mid-run. 1.05 absorbs slop without 2x'ing
# SSP-bound pre-mint time.
INVOICE_SAFETY="${INVOICE_SAFETY:-1.05}"
TREASURER_BUFFER="${TREASURER_BUFFER:-1.5}"
# Floor: even info/receive-only sweeps need a small treasurer balance
# for the treasurer wallet to exist & for /receive to point somewhere.
TREASURER_MIN_FLOOR="${TREASURER_MIN_FLOOR:-50000}"
# Lightning knobs: only used when the mix contains a send_ln op. The
# pre-mint step (Main.kt mint-invoices) mints a pool of fixed-amount,
# long-expiry bolt11 invoices across BANKS receiver wallets and probes
# the SSP for `lightningFeeSats` once. Sender prefunding then gets
# `(payment_sats + ln_fee × LN_FEE_SAFETY)` headroom per send.
BANKS="${BANKS:-50}"
INVOICE_EXPIRY_SECS="${INVOICE_EXPIRY_SECS:-604800}"   # 7 days
MINT_PARALLELISM="${MINT_PARALLELISM:-20}"
LN_FEE_SAFETY="${LN_FEE_SAFETY:-1.5}"

# Detect whether the mix uses LN sends / spark sends. This drives:
# (a) whether we run mint-invoices + size sender prefunding for LN
#     fee leakage; (b) whether we run the treasurer-addr bootstrap
# (skip the multi-minute cold-start if no spark-send needs the addr).
detect_mix_kinds() {
    python3 -c '
import sys
labels = {e.split("=")[0].strip() for e in sys.argv[1].split(",") if "=" in e}
has_ln = bool(labels & {"send_ln", "send_lightning", "send_bolt11"})
has_spark = bool(labels & {"send", "send_spark"})
print(int(has_ln), int(has_spark))
' "$1"
}
read SEND_LN_PRESENT SPARK_SEND_PRESENT < <(detect_mix_kinds "$MIX")
echo "[sweep] mix kinds: send_ln=$SEND_LN_PRESENT  send_spark=$SPARK_SEND_PRESENT"

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

compute_funding() {
    # Echoes "PER_SENDER TREASURER INVOICE_COUNT" — 3 ints.
    python3 "$SCRIPT_DIR/compute_funding.py" \
        --rps "$SWEEP_RPS" \
        --duration "$DURATION" \
        --mix "$MIX" \
        --senders "$SENDERS" \
        --payment-sats "$PAYMENT_SATS" \
        --safety "$SEED_SAFETY" \
        --invoice-safety "$INVOICE_SAFETY" \
        --buffer "$TREASURER_BUFFER" \
        --floor "$TREASURER_MIN_FLOOR" \
        --ln-fee-sats "$1"
}

# Pass 1: ln_fee=0 (just to discover INVOICE_COUNT, which is independent
# of fee). For spark-only mixes this is the only pass and INVOICE_COUNT=0.
read PER_SENDER_SATS TREASURER_TARGET INVOICE_COUNT < <(compute_funding 0)
LN_FEE_SATS=0
INVOICE_POOL_FILE=""

if [ "$SEND_LN_PRESENT" = "1" ]; then
    if [ "$INVOICE_COUNT" -le 0 ]; then
        echo "[sweep] mix has send_ln but compute_funding returned invoice_count=0 — aborting"
        exit 2
    fi
    INVOICE_POOL_FILE="$SWEEP_DIR/invoices.txt"
    echo
    echo "[sweep] === mint-invoices (count=$INVOICE_COUNT, banks=$BANKS) ==="
    mint_log="$SWEEP_DIR/mint.log"
    stream_filtered "$mint_log" '^\[mint\]' \
        ./gradlew run --console=plain --args="--mode=mint-invoices \
        --mysql-url=$MYSQL_URL \
        --master-secret=$MASTER_SECRET \
        --count=$INVOICE_COUNT \
        --amount-sats=$PAYMENT_SATS \
        --banks=$BANKS \
        --expiry-secs=$INVOICE_EXPIRY_SECS \
        --parallelism=$MINT_PARALLELISM \
        --out=$INVOICE_POOL_FILE"
    if [ "$STREAM_RC" -ne 0 ]; then
        echo "[sweep] mint-invoices failed (rc=$STREAM_RC); see $mint_log"
        exit "$STREAM_RC"
    fi
    # Probed SSP fee. Read from the sidecar mint-invoices writes
    # (<pool>.fee) — the same file compute_funding.py picks up as a
    # fallback. Multiplied by LN_FEE_SAFETY in the math so a fee bump
    # mid-run doesn't drain a sender.
    FEE_FILE="$INVOICE_POOL_FILE.fee"
    if [ ! -s "$FEE_FILE" ]; then
        echo "[sweep] mint-invoices did not write $FEE_FILE — aborting"
        exit 2
    fi
    PROBED_FEE=$(cat "$FEE_FILE")
    LN_FEE_SATS=$(python3 -c "import math,sys; print(math.ceil(int(sys.argv[1]) * float(sys.argv[2])))" "$PROBED_FEE" "$LN_FEE_SAFETY")
    echo "[sweep] probed ln_fee_sats=$PROBED_FEE  × safety=$LN_FEE_SAFETY → ${LN_FEE_SATS}sat per send_ln"
    # Pass 2: re-run with the real fee so per-sender drain reflects LN leakage.
    read PER_SENDER_SATS TREASURER_TARGET _ < <(compute_funding "$LN_FEE_SATS")
fi

echo "[sweep] computed funding budget:"
echo "[sweep]   per-sender    = ${PER_SENDER_SATS} sats  (drain × safety=${SEED_SAFETY}, ln_fee=${LN_FEE_SATS})"
echo "[sweep]   treasurer     = ${TREASURER_TARGET} sats (K × per-sender × buffer=${TREASURER_BUFFER}, floor=${TREASURER_MIN_FLOOR})"
echo "[sweep]   invoice_count = ${INVOICE_COUNT}"

# Manifest captures the inputs so the aggregator (and a partner reader)
# can reconstruct what the sweep was. Written before any steps run so
# even an aborted sweep leaves provenance behind.
cat > "$SWEEP_DIR/manifest.json" <<EOF
{
  "sweep_id": "$SWEEP_ID",
  "master_secret": "$MASTER_SECRET",
  "rps_steps": [$(echo "$SWEEP_RPS" | sed 's/,/, /g')],
  "duration_per_step": "$DURATION",
  "mix": "$MIX",
  "users": $USERS,
  "senders": $SENDERS,
  "distribution": "$DIST",
  "payment_sats": $PAYMENT_SATS,
  "port": $PORT,
  "send_ln_present": $SEND_LN_PRESENT,
  "spark_send_present": $SPARK_SEND_PRESENT,
  "lightning": {
    "banks": $BANKS,
    "invoice_count": $INVOICE_COUNT,
    "invoice_expiry_secs": $INVOICE_EXPIRY_SECS,
    "ln_fee_sats": $LN_FEE_SATS,
    "ln_fee_safety": $LN_FEE_SAFETY
  },
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
# Skipped for pure-LN / info+receive-only sweeps — the treasurer is never
# a send destination there, and the bootstrap pays multi-minute cold-start
# sync the first time per master secret.
if [ "$SPARK_SEND_PRESENT" = "1" ]; then
    ensure_treasurer_addr || exit 1
else
    echo "[sweep] mix has no spark-send op — skipping treasurer-addr bootstrap"
    TREASURER_ADDR=""
fi

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
        ${LOG_FILTER:+--log-filter=$LOG_FILTER} --bench-trace=$([ "$BENCH_TRACE" = 0 ] && echo false || echo true)" \
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
    # Build the optional flags (treasurer addr only when spark-send is in
    # the mix; invoice pool only when send_ln is). Empty values would
    # confuse the loadgen arg parser, so omit them entirely instead.
    LOADGEN_OPT_ARGS=""
    if [ -n "$TREASURER_ADDR" ]; then
        LOADGEN_OPT_ARGS="$LOADGEN_OPT_ARGS --treasurer-spark-addr=$TREASURER_ADDR"
    fi
    if [ -n "$INVOICE_POOL_FILE" ]; then
        LOADGEN_OPT_ARGS="$LOADGEN_OPT_ARGS --invoice-pool=$INVOICE_POOL_FILE"
    fi
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
        --out-dir=$step_dir$LOADGEN_OPT_ARGS"
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

# Post-run bolt11 audit. send_ln dispatches return on SSP-accept, which
# is well before LN settlement, so client_ok in latency.jsonl can mask
# a tail of unsettled payments. The audit syncs each sender wallet and
# classifies every dispatched invoice via listPayments; aggregate.py
# picks up audit.json and adds a "Lightning settlement audit" section
# to RESULTS.md. Skipped for spark-only sweeps (no audit relevance).
if [ "$SEND_LN_PRESENT" = "1" ]; then
    echo "[sweep] === audit-bolt11 (validate settlement on sender wallets) ==="
    audit_log="$SWEEP_DIR/audit.log"
    AUDIT_PARALLELISM="${AUDIT_PARALLELISM:-5}"
    stream_filtered "$audit_log" '^\[audit\]' \
        ./gradlew run --console=plain --args="--mode=audit-bolt11 \
        --mysql-url=$MYSQL_URL \
        --master-secret=$MASTER_SECRET \
        --sweep-dir=$SWEEP_DIR \
        --parallelism=$AUDIT_PARALLELISM"
    if [ "$STREAM_RC" -ne 0 ]; then
        echo "[sweep] audit-bolt11 failed (rc=$STREAM_RC); see $audit_log — RESULTS.md will lack the audit section"
    fi
fi

echo "[sweep] done. Steps in $SWEEP_DIR/. Aggregate with: make aggregate SWEEP_ID=$SWEEP_ID"
