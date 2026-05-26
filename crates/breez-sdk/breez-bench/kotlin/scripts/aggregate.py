#!/usr/bin/env python3
"""Aggregate an RPS sweep into summary.json + RESULTS.md.

Walks `out/<sweep-id>/rps-<N>/` directories, reads `latency.jsonl`
(client-side observed timings), `requests.jsonl` (server-side handler
timings), and `metrics.jsonl` (1Hz process samples). Stdlib only.
"""

import argparse
import json
import math
import re
import sys
from pathlib import Path


# --- error categorization -------------------------------------------------

# Order matters: most specific patterns first. The error string we get
# back is usually the full chained Rust error, so port-exhaustion or
# MySQL-pool-exhausted root causes show up nested inside SparkException
# / StorageException — we want to attribute to the root, not the wrapper,
# so the root-cause patterns are checked before the wrapper types.
ERROR_PATTERNS = [
    ("mysql_pool_exhausted", re.compile(r"Too many connections|ERROR HY000 \(1040\)")),
    ("port_exhaustion",      re.compile(r"Can't assign requested address|os error 49")),
    ("connect_timeout",      re.compile(r"Operation timed out|os error 60")),
    ("operator_unreachable", re.compile(r"Operator RPC error|tcp connect error|status:\s*Unavailable")),
    ("operator_other",       re.compile(r"SparkException|Tree service error")),
    ("storage_other",        re.compile(r"StorageException")),
    ("request_timeout",      re.compile(r"HttpTimeoutException|request timed out|timed out")),
    ("connect_refused",      re.compile(r"ConnectException|Connection refused")),
    ("http_5xx",             re.compile(r"^http_5\d\d$")),
    ("http_4xx",             re.compile(r"^http_4\d\d$")),
]


def categorize_error(err):
    """Bucket a raw error string into a category. None → no error."""
    if not err:
        return None
    for name, pattern in ERROR_PATTERNS:
        if pattern.search(err):
            return name
    return "other"


def bucket_errors(rows):
    """Returns {category: count} from rows that have a non-null error.
    Drops `dropped` rows (those are loadgen-side over-cap, not server failures)."""
    out = {}
    for r in rows:
        if r.get("dropped"):
            continue
        cat = categorize_error(r.get("error"))
        if cat is None:
            continue
        out[cat] = out.get(cat, 0) + 1
    return out




# --- I/O helpers ----------------------------------------------------------

def read_jsonl(path):
    rows = []
    if not path.exists():
        return rows
    with path.open("r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError as e:
                print(f"warn: skipping malformed line in {path}: {e}", file=sys.stderr)
    return rows


# --- trace log parsing ----------------------------------------------------
# Per-step server writes its tracing output to <step>/.trace-logs/sdk.log
# when started with --bench-trace (or an explicit --log-filter). We parse
# two event categories out of it:
#
#   1. "Swapped leaves to match target amount" message
#      → payment-time leaf swap (one per send that required a swap)
#   2. Span CLOSE events on the SDK span-trace targets:
#        - `breez_sdk_core::send_payment` — top-level `send_payment`
#        - `spark::ssp`                   — every SSP method
#        - `spark::operator_rpc`          — every operator-RPC method
#      Each crate uses its own target to avoid the spark crate
#      referencing a breez-owned name. The span hierarchy carries
#      `payment_id` (top-level span field) and, for operator RPCs,
#      `operator_id`. Format produced by `FmtSpan::CLOSE`:
#
#          <ts>  INFO send_payment{payment_id="..."}:rpc_name{operator_id=3}:
#                spark::operator_rpc: close time.busy=2.53ms time.idle=24.9µs
#
# The sdk.log file is naturally per-step (each step gets a fresh server),
# so no timestamp filtering is needed inside this parser.

SWAP_MESSAGE = "Swapped leaves to match target amount"
# Must stay in sync with the SDK targets and the bench filter the server
# installs — see `BENCH_TRACE_FILTER` in Server.kt and `SPAN_TRACE_TARGETS`
# in core/src/logger.rs. Changes to a target name, the tracing level, or
# the close-event format need to land on all three sides.
BENCH_TARGETS = (
    "breez_sdk_core::send_payment",
    "spark::ssp",
    "spark::operator_rpc",
)
TOP_LEVEL_SPAN_NAME = "send_payment"

# Matches one CLOSE event. The span hierarchy is captured as the
# colon-separated run of `name{fields}` segments preceding the target
# marker. Field values are either quoted strings (`payment_id="abc"`)
# or bare tokens (`operator_id=3`). The optional `<lineno>:` between
# target and `close` is rendered by tracing-subscriber when the SDK's
# fmt layer is configured with `with_line_number(true)` — that's the
# prod default, which the simplified `init_logging` reuses for the
# bench build too.
_CLOSE_RE = re.compile(
    r"^\S+\s+INFO\s+(?P<hierarchy>\S+?):\s+"
    + r"(?:" + "|".join(re.escape(t) for t in BENCH_TARGETS) + r")"
    + r":\s+(?:\d+:\s+)?close\s+time\.busy=(?P<busy>[\d.]+(?:ns|µs|us|ms|s))"
    + r"(?:\s+time\.idle=(?P<idle>[\d.]+(?:ns|µs|us|ms|s)))?"
)

# Within a hierarchy segment: `name` or `name{field=value, ...}`.
_SEGMENT_RE = re.compile(r"(\w+)(?:\{([^}]*)\})?")
_FIELD_RE = re.compile(r'(\w+)=("[^"]*"|[^\s,}]+)')
_DURATION_RE = re.compile(r"^([\d.]+)(ns|µs|us|ms|s)$")
_DURATION_UNIT_MS = {
    "ns": 1e-6,
    "µs": 1e-3,
    "us": 1e-3,
    "ms": 1.0,
    "s": 1000.0,
}


def _parse_duration_ms(s):
    if not s:
        return None
    m = _DURATION_RE.match(s)
    if not m:
        return None
    return float(m.group(1)) * _DURATION_UNIT_MS[m.group(2)]


def _parse_fields(s):
    """Returns {field → value} where quoted values keep their quotes
    stripped. Bare numeric / identifier values are returned as strings;
    the caller converts where needed."""
    out = {}
    for k, v in _FIELD_RE.findall(s):
        out[k] = v.strip('"') if v.startswith('"') else v
    return out


def _parse_hierarchy(s):
    """Splits a span hierarchy into ordered [{name, fields}, ...]
    segments. Splits on top-level `:` only — field bodies may contain
    them (in quoted strings), so we track `{}` depth."""
    pieces = []
    depth = 0
    start = 0
    for i, ch in enumerate(s):
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
        elif ch == ":" and depth == 0:
            pieces.append(s[start:i])
            start = i + 1
    pieces.append(s[start:])
    out = []
    for piece in pieces:
        m = _SEGMENT_RE.match(piece)
        if not m:
            continue
        out.append({
            "name": m.group(1),
            "fields": _parse_fields(m.group(2) or ""),
        })
    return out


def parse_trace_log(path):
    """Returns swap count and the list of parsed close events.

    Phase events: list of dicts each carrying
      {payment_id, rpc_name, operator_id (or None), duration_ms}.
    Top-level `send_payment` spans are kept (they're the per-payment
    total elapsed time) — callers distinguish them by `rpc_name`.
    Missing file ⇒ zero + empty list, which is also the
    'server ran without --bench-trace' signal further up.
    """
    out = {"swap_count": 0, "phase_events": []}
    if not path.exists():
        return out
    with path.open("r", encoding="utf-8", errors="replace") as f:
        for line in f:
            if SWAP_MESSAGE in line:
                out["swap_count"] += 1
            m = _CLOSE_RE.match(line)
            if not m:
                continue
            busy_ms = _parse_duration_ms(m.group("busy"))
            idle_ms = _parse_duration_ms(m.group("idle")) or 0.0
            if busy_ms is None:
                continue
            segments = _parse_hierarchy(m.group("hierarchy"))
            if not segments:
                continue
            # Walk the hierarchy to find payment_id (set by the
            # top-level `send_payment` span). Missing ⇒ orphaned event
            # (instrumented code ran outside a send_payment call,
            # e.g. claim flows); skip it for the slow-payment view.
            payment_id = None
            for seg in segments:
                pid = seg["fields"].get("payment_id")
                if pid:
                    payment_id = pid
                    break
            if payment_id is None:
                continue
            leaf = segments[-1]
            op_raw = leaf["fields"].get("operator_id")
            try:
                operator_id = int(op_raw) if op_raw is not None else None
            except ValueError:
                operator_id = None
            out["phase_events"].append({
                "payment_id": payment_id,
                "rpc_name": leaf["name"],
                "operator_id": operator_id,
                "duration_ms": busy_ms + idle_ms,
            })
    return out


# --- stats ---------------------------------------------------------------

def percentile(sorted_values, p):
    """Linear-interpolated percentile in [0, 100]. Returns None for empty input."""
    if not sorted_values:
        return None
    if len(sorted_values) == 1:
        return float(sorted_values[0])
    k = (len(sorted_values) - 1) * (p / 100.0)
    f = math.floor(k)
    c = min(f + 1, len(sorted_values) - 1)
    if f == c:
        return float(sorted_values[f])
    return float(sorted_values[f] + (sorted_values[c] - sorted_values[f]) * (k - f))


def summary_stats(values):
    if not values:
        return {"count": 0}
    sv = sorted(values)
    return {
        "count": len(sv),
        "min": sv[0],
        "max": sv[-1],
        "mean": sum(sv) / len(sv),
        "p50": percentile(sv, 50),
        "p95": percentile(sv, 95),
        "p99": percentile(sv, 99),
    }


# --- slow-payment phase helpers -----------------------------------------

def _phases_by_payment(phase_events):
    """Index parsed close events by payment_id. Returns
    {payment_id → list[event]}, preserving insertion order."""
    out = {}
    for ev in phase_events:
        out.setdefault(ev["payment_id"], []).append(ev)
    return out


def _payment_summary(events):
    """Per-RPC summary for one payment. Operator RPCs fan out to N
    operators concurrently, so each rpc_name gets a list of durations.
    Returns (per_rpc: {rpc_name → list[(operator_id, ms)]}, total_ms).
    """
    per_rpc = {}
    total_ms = None
    for ev in events:
        if ev["rpc_name"] == TOP_LEVEL_SPAN_NAME:
            # Top-level `send_payment` span — record as the wall-clock
            # total but don't list it as an RPC row.
            total_ms = ev["duration_ms"]
            continue
        per_rpc.setdefault(ev["rpc_name"], []).append(
            (ev["operator_id"], ev["duration_ms"])
        )
    return per_rpc, total_ms


def _dominant_rpc(per_rpc):
    """Returns (rpc_name, worst_ms) — the slowest individual RPC call
    in the payment (max across all operator fan-outs). None if the
    payment has no RPC data (unlikely but possible)."""
    best = None
    best_ms = -1.0
    for rpc, calls in per_rpc.items():
        for _, ms in calls:
            if ms > best_ms:
                best_ms = ms
                best = rpc
    return (best, best_ms) if best is not None else None


def _slow_phase_aggregate(slow_rows, phase_by_payment):
    """Per-RPC aggregate over the slow set. For each rpc_name, gather
    every individual call (across all payments and all operators), then
    compute summary_stats. Also tracks dominance: how many slow
    payments had this RPC as their slowest individual call.
    """
    per_rpc_ms = {}
    dominance = {}
    classified = 0
    for r in slow_rows:
        pid = r.get("payment_id")
        if not pid:
            continue
        events = phase_by_payment.get(pid)
        if not events:
            continue
        per_rpc, _ = _payment_summary(events)
        if not per_rpc:
            continue
        classified += 1
        for rpc, calls in per_rpc.items():
            ms_list = per_rpc_ms.setdefault(rpc, [])
            for _, ms in calls:
                ms_list.append(ms)
        dom = _dominant_rpc(per_rpc)
        if dom:
            dominance[dom[0]] = dominance.get(dom[0], 0) + 1
    out = {}
    for rpc, ms_list in per_rpc_ms.items():
        stats = summary_stats(ms_list)
        dpct = (100.0 * dominance.get(rpc, 0) / classified) if classified else None
        out[rpc] = {**stats, "dominant_pct": dpct}
    out["_classified"] = classified
    return out


# --- per-step aggregation ------------------------------------------------

SUBTIMING_FIELDS = ("build_ms", "prepare_ms", "send_ms", "op_ms", "disconnect_ms")


def per_op_subtimings(rows, op_field="op"):
    """Per-op sub-timing breakdown from requests.jsonl (server-side).

    Returns dict[op] → dict[field] → summary_stats. Splits a send_ln's
    duration into where it actually went (build / prepare / send /
    disconnect). The dominant column is the bottleneck row in RESULTS.

    Errored rows are excluded so the breakdown reflects the successful
    flow only; a partial-error row's sub-timings would conflate the
    pre-error phase with `null` for the post-error phases.
    """
    by_op_field = {}
    for r in rows:
        if r.get("error") is not None:
            continue
        op = r.get(op_field)
        if op is None:
            continue
        for f in SUBTIMING_FIELDS:
            v = r.get(f)
            if v is None:
                continue
            by_op_field.setdefault(op, {}).setdefault(f, []).append(v)
    out = {}
    for op, fields in by_op_field.items():
        out[op] = {f: summary_stats(v) for f, v in fields.items()}
    return out


def per_op_latency(rows, duration_field, op_field):
    """Group durations by op, return summary stats per op.

    Also emits synthetic `_send_rollup` / `_receive_rollup` entries: the
    union of all `send*` / `receive*` durations. These power the
    headline `max_safe_rps` metric (defined on the send class as a
    whole — a `send_ln` is ~2 SSP roundtrips vs a local Spark transfer,
    so blending them into one literal `send` p99 would be bimodal junk;
    splitting them across rows is right for the table, rolling them up
    is right for the cliff metric). Underscore-prefixed so the per-op
    table renderer skips them.
    """
    by_op = {}
    for r in rows:
        if r.get("error") is not None:
            continue
        if r.get("dropped"):
            continue
        d = r.get(duration_field)
        if d is None:
            continue
        op = r.get(op_field)
        if op is None:
            continue
        by_op.setdefault(op, []).append(d)
    result = {op: summary_stats(v) for op, v in by_op.items()}
    send_durs = [d for op, ds in by_op.items() if op.startswith("send") for d in ds]
    if send_durs:
        result["_send_rollup"] = summary_stats(send_durs)
    receive_durs = [d for op, ds in by_op.items() if op.startswith("receive") for d in ds]
    if receive_durs:
        result["_receive_rollup"] = summary_stats(receive_durs)
    return result


def metrics_window(metrics_rows, ts_lo, ts_hi):
    """Slice metrics samples to [ts_lo, ts_hi]; -1 sentinels are filtered."""
    rss, heap_used, mysql, sockets, fds, threads = [], [], [], [], [], []
    proc_cpu, host_cpu = [], []
    cpu_count = -1
    for m in metrics_rows:
        ts = m.get("ts")
        if ts is None or ts < ts_lo or ts > ts_hi:
            continue
        for src, dst in (
            ("rss_kb", rss),
            ("heap_used_bytes", heap_used),
            ("mysql_conns", mysql),
            ("remote_tcp_sockets", sockets),
            ("fd_count", fds),
            ("thread_count", threads),
            ("process_cpu_load", proc_cpu),
            ("host_cpu_load", host_cpu),
        ):
            v = m.get(src)
            # CPU loads are floats in [0,1]; -1.0 is the sentinel like
            # the integer -1 sentinel used elsewhere.
            if v is not None and v >= 0:
                dst.append(v)
        ap = m.get("available_processors")
        if ap is not None and ap > 0:
            cpu_count = ap
    return {
        "rss_kb": summary_stats(rss),
        "heap_used_bytes": summary_stats(heap_used),
        "mysql_conns": summary_stats(mysql),
        "remote_tcp_sockets": summary_stats(sockets),
        "fd_count": summary_stats(fds),
        "thread_count": summary_stats(threads),
        "process_cpu_load": summary_stats(proc_cpu),
        "host_cpu_load": summary_stats(host_cpu),
        "available_processors": cpu_count,
    }


def aggregate_step(step_dir, slow_abs_ms=2000, slow_rel_pct=95, slow_top_n=20):
    """Aggregate one rps-N directory. Returns a dict (or None if no data).

    Slow-payment knobs are forwarded so the per-step slow set + Top-N
    table can be computed while the raw rows are still in scope.
    """
    latency_rows = read_jsonl(step_dir / "latency.jsonl")
    requests_rows = read_jsonl(step_dir / "requests.jsonl")
    metrics_rows = read_jsonl(step_dir / "metrics.jsonl")
    trace = parse_trace_log(step_dir / ".trace-logs" / "sdk.log")

    if not latency_rows and not requests_rows:
        return None

    # Step time bounds: full window from first observation to last.
    all_ts = (
        [r["ts"] for r in latency_rows if "ts" in r]
        + [r["ts"] for r in requests_rows if "ts" in r]
    )
    ts_lo = min(all_ts) if all_ts else 0
    ts_hi = max(all_ts) if all_ts else ts_lo

    total_dispatched = sum(1 for r in latency_rows)
    total_dropped = sum(1 for r in latency_rows if r.get("dropped"))
    total_errors = sum(
        1 for r in latency_rows if r.get("error") is not None and not r.get("dropped")
    )
    # Error breakdown by category. Server-side is the more useful view
    # (actual exception types from the SDK); client-side is mostly
    # transport-layer issues + http_5xx wrappers around server errors.
    client_errors_by_category = bucket_errors(latency_rows)
    server_errors_by_category = bucket_errors(requests_rows)

    # Client outcome split. A 60s client timeout (LoadGen.kt) is NOT a
    # server failure — at high RPS the server often completes the request
    # well after the client gave up. Keep `timed_out` distinct from real
    # `failed` so the report can't conflate "too slow for the client" with
    # "the operation failed".
    client_timed_out = client_errors_by_category.get("request_timeout", 0)
    client_failed = total_errors - client_timed_out
    client_ok = total_dispatched - total_dropped - total_errors

    # Server-side counts come from requests.jsonl (handler invocations the
    # server actually logged) — NOT from the client's dispatched/errors.
    # `server_completed` < `dispatched` at collapse because many dispatched
    # requests never reach a completed handler within the window.
    server_completed = len(requests_rows)
    server_err = sum(1 for r in requests_rows if r.get("error") is not None)
    server_ok = server_completed - server_err

    client_lat = per_op_latency(latency_rows, duration_field="duration_ms", op_field="op")
    server_lat = per_op_latency(requests_rows, duration_field="duration_ms", op_field="op")
    server_subtimings = per_op_subtimings(requests_rows, op_field="op")

    metrics = metrics_window(metrics_rows, ts_lo, ts_hi) if metrics_rows else {}

    # Leaf-swap rate: denominator is successful send-class handler
    # completions for this step. Counts come from the per-step trace
    # log (only populated under --bench-trace). Zero count with the
    # file absent ⇒ render section will be skipped.
    send_ok_rows = [
        r for r in requests_rows
        if r.get("error") is None and (r.get("op") or "").startswith("send")
    ]
    send_ok_count = len(send_ok_rows)
    swap_count = trace["swap_count"]
    swap_rate = (swap_count / send_ok_count) if send_ok_count else None

    # Slow-payment phase breakdown. Threshold is
    # `max(slow_abs_ms, step_p_rel)` so we always honour the absolute
    # floor (catches the pathological tails) while the relative cap
    # keeps the slow set bounded at high RPS.
    send_durations_ok = [
        r.get("duration_ms") for r in send_ok_rows if r.get("duration_ms") is not None
    ]
    if send_durations_ok:
        step_p_rel = percentile(sorted(send_durations_ok), slow_rel_pct)
        slow_threshold_ms = max(slow_abs_ms, step_p_rel or 0)
    else:
        step_p_rel = None
        slow_threshold_ms = slow_abs_ms
    slow_rows = [
        r for r in send_ok_rows
        if (r.get("duration_ms") or 0) >= slow_threshold_ms
    ]
    slow_rows.sort(key=lambda r: r.get("duration_ms") or 0, reverse=True)

    phase_by_payment = _phases_by_payment(trace["phase_events"])
    slow_top_n_rows = []
    for r in slow_rows[:slow_top_n]:
        pid = r.get("payment_id")
        events = phase_by_payment.get(pid, []) if pid else []
        per_rpc, span_total_ms = _payment_summary(events) if events else ({}, None)
        dom = _dominant_rpc(per_rpc) if per_rpc else None
        slow_top_n_rows.append({
            "payment_id": pid,
            "op": r.get("op"),
            "duration_ms": r.get("duration_ms"),
            "span_total_ms": span_total_ms,
            "per_rpc": {
                # Per-RPC summary: count, sum, max (worst-of-N for
                # operator RPCs), and the operator id of the worst
                # call when applicable.
                rpc: {
                    "count": len(calls),
                    "sum_ms": sum(ms for _, ms in calls),
                    "max_ms": max(ms for _, ms in calls),
                    "worst_operator_id": next(
                        (op for op, ms in calls if ms == max(m for _, m in calls)),
                        None,
                    ),
                }
                for rpc, calls in per_rpc.items()
            },
            "dominant_rpc": dom[0] if dom else None,
            "dominant_ms": dom[1] if dom else None,
        })
    # Per-RPC aggregate over the slow set.
    slow_phase_agg = _slow_phase_aggregate(slow_rows, phase_by_payment)

    return {
        "ts_window_ms": [ts_lo, ts_hi],
        "total_dispatched": total_dispatched,
        "total_dropped": total_dropped,
        "total_errors_post_dispatch": total_errors,
        "client_ok": client_ok,
        "client_timed_out": client_timed_out,
        "client_failed": client_failed,
        "server_completed": server_completed,
        "server_ok": server_ok,
        "server_err": server_err,
        "client_latency_ms": client_lat,
        "server_latency_ms": server_lat,
        "server_subtimings_ms": server_subtimings,
        "metrics": metrics,
        "errors_by_category_client": client_errors_by_category,
        "errors_by_category_server": server_errors_by_category,
        "send_ok_count": send_ok_count,
        "swap_count": swap_count,
        "swap_rate": swap_rate,
        "slow_threshold_ms": slow_threshold_ms,
        "slow_abs_ms": slow_abs_ms,
        "slow_rel_pct": slow_rel_pct,
        "slow_rel_p_value_ms": step_p_rel,
        "slow_count": len(slow_rows),
        "slow_top_n": slow_top_n_rows,
        "slow_phase_aggregate": slow_phase_agg,
    }


# --- sweep-wide derivations ----------------------------------------------

def step_dirs(sweep_dir):
    """Return rps-N subdirs sorted by N ascending."""
    out = []
    for child in sweep_dir.iterdir():
        if child.is_dir() and child.name.startswith("rps-"):
            try:
                rps = int(child.name[len("rps-"):])
            except ValueError:
                continue
            out.append((rps, child))
    return sorted(out, key=lambda t: t[0])


def derive_p99_doubling(steps_summary):
    """Find the highest RPS at which client-side p99(send) is still < 2×
    the smallest stable p99(send). 'Smallest stable' = the lowest p99 across
    the swept steps where send had ≥30 samples (so p99 is meaningful).

    Returns (max_safe_rps, baseline_rps, baseline_p99, doubling_rps_or_none).
    """
    candidates = []
    for rps, s in steps_summary:
        # Collapsed steps have survivorship-biased latency (only the
        # sub-60s-timeout survivors are sampled), so their p99 is
        # meaningless here and could inflate max_safe or even become the
        # baseline. The headline labels this "pre-collapse steps only" —
        # enforce that.
        if step_state(s) == "collapsed":
            continue
        # Use the synthetic send-class rollup (union of send / send_spark
        # / send_ln durations). per_op_latency emits this whenever any
        # send* op has data, so it's always present when there's send data
        # to consider.
        lat = s.get("client_latency_ms", {})
        send = lat.get("_send_rollup")
        if not send or send.get("count", 0) < 30:
            continue
        candidates.append((rps, send["p99"]))
    if not candidates:
        return None, None, None, None

    baseline = min(p99 for _, p99 in candidates)
    baseline_rps = next(rps for rps, p99 in candidates if p99 == baseline)
    threshold = 2 * baseline

    max_safe = None
    crossed_at = None
    for rps, p99 in candidates:
        if p99 < threshold:
            if max_safe is None or rps > max_safe:
                max_safe = rps
        elif crossed_at is None or rps < crossed_at:
            crossed_at = rps
    return max_safe, baseline_rps, baseline, crossed_at


# --- collapse classification ---------------------------------------------
# Client goodput (client_ok / dispatched) thresholds. Below COLLAPSE the
# step is in congestion collapse: its latency percentiles are
# survivorship-biased (only the lucky sub-60s survivors counted) and its
# step-to-step ordering is NOISE — driven by the external shared operator
# rate limiter's time-varying state, closed-loop funding feedback (failed
# sends don't drain senders, so a worse step can enable a better next
# one), and per-step SIGKILL truncating in-flight retry storms. Collapsed
# steps must not be compared to each other or read as measurements.
OK_GOODPUT = 0.95
COLLAPSE_GOODPUT = 0.50


def step_state(s):
    """Classify a step from client goodput: ok | degrading | collapsed."""
    disp = s.get("total_dispatched", 0)
    if disp <= 0:
        return "collapsed"
    gp = s.get("client_ok", 0) / disp
    if gp >= OK_GOODPUT:
        return "ok"
    if gp >= COLLAPSE_GOODPUT:
        return "degrading"
    return "collapsed"


# --- markdown rendering --------------------------------------------------

def fmt_ms(v):
    return "—" if v is None else f"{v:.1f}"


def fmt_kb_as_mib(v):
    if v is None:
        return "—"
    return f"{v / 1024.0:.1f}"


def render_table(headers, rows):
    """Render a markdown table with cells padded to a consistent column
    width so the raw source is also readable (not just the GitHub render).

    Width = max(len(header), max(len(cell))) per column, with at least 3
    dashes in the separator (markdown spec minimum). All cells are
    right-padded with spaces. Returns a list of lines (header, sep, …rows).
    """
    cols = list(zip(headers, *rows)) if rows else [(h,) for h in headers]
    widths = [max(len(str(x)) for x in col) for col in cols]
    sep_widths = [max(w, 3) for w in widths]

    def fmt_row(values):
        return "| " + " | ".join(str(v).ljust(w) for v, w in zip(values, widths)) + " |"

    out = [fmt_row(headers)]
    out.append("|" + "|".join("-" * (w + 2) for w in sep_widths) + "|")
    for row in rows:
        out.append(fmt_row(row))
    return out


def render_swap_section(steps_summary):
    """Per-step leaves-swap rate. Counts come from `.trace-logs/sdk.log`
    (only populated when the server ran with `--bench-trace`). Section
    is skipped if no swap events were seen across any step — keeps the
    report quiet for runs that didn't ask for the data.
    """
    has_any = any(s.get("swap_count", 0) for _, s in steps_summary)
    if not has_any:
        return []
    out = ["## Leaves swap (per step)", ""]
    out.append(
        "> Counted from `.trace-logs/sdk.log` — populated when the "
        "server ran with `--bench-trace`. Denominator is the step's "
        "successful `send*` handler completions; high swap rate at a "
        "given RPS implies the leaf set isn't pre-shaped for the "
        "payment denominations being sent."
    )
    out.append("")
    headers = ["RPS", "successful sends", "swaps", "swap rate"]
    rows = []
    for rps, s in steps_summary:
        sok = s.get("send_ok_count", 0)
        sr = s.get("swap_rate")
        rows.append([
            str(rps),
            str(sok),
            str(s.get("swap_count", 0)),
            "—" if sr is None else f"{100.0 * sr:.1f}%",
        ])
    out.extend(render_table(headers, rows))
    out.append("")
    return out


def render_slow_payments_section(steps_summary):
    """Per-step slow-payment phase breakdown. For every slow send we
    list its slowest individual RPC call (with operator_id when
    applicable). A second sub-table aggregates per-RPC stats across
    the whole slow set so a recurring bottleneck (e.g. one SSP method
    always > 1s) jumps out.

    Section is skipped if no phase events were parsed across any step
    (typically: ran without `--bench-trace`).
    """
    has_phase_data = any(
        any(t.get("per_rpc") for t in s.get("slow_top_n", []))
        for _, s in steps_summary
    )
    if not has_phase_data:
        return []
    first = steps_summary[0][1] if steps_summary else {}
    abs_ms = first.get("slow_abs_ms", "?")
    rel_pct = first.get("slow_rel_pct", "?")
    out = [
        "## Slow payments — per-RPC breakdown (per step)",
        "",
        (
            f"> A send is 'slow' if `duration_ms ≥ max({abs_ms}ms, "
            f"step p{rel_pct})` (override via `--slow-abs-ms` / "
            f"`--slow-rel-pct`). Per-RPC timings come from "
            "`#[tracing::instrument]`-decorated SSP and operator-RPC "
            "methods; correlation to server log rows is by `payment_id` "
            "carried on the top-level `send_payment` span. The "
            "**dominant** column is the slowest individual RPC call in "
            "the payment — for operator RPCs that includes a "
            "`(so-N)` tag identifying which of the parallel "
            "operators held up the FROST round. Empty `per_rpc` (rendered "
            "as `—`) means the trace events were missed or the server "
            "ran without `--bench-trace`."
        ),
        "",
    ]
    for rps, s in steps_summary:
        if step_state(s) == "collapsed":
            continue
        slow_threshold_ms = s.get("slow_threshold_ms")
        slow_count = s.get("slow_count", 0)
        top_n = s.get("slow_top_n", [])
        agg = s.get("slow_phase_aggregate") or {}
        if not top_n:
            continue
        out.append(
            f"### RPS {rps} — slow threshold {slow_threshold_ms:.0f}ms "
            f"({slow_count} slow send{'s' if slow_count != 1 else ''})"
        )
        out.append("")
        out.append("#### Top slowest sends")
        out.append("")
        headers = [
            "payment_id", "op", "total ms", "span total ms",
            "dominant RPC", "dominant ms", "# RPCs",
        ]
        rows = []
        for t in top_n:
            dom_rpc = t.get("dominant_rpc")
            per_rpc = t.get("per_rpc") or {}
            # For operator-fanout RPCs, surface which operator was
            # slowest so the Spark team can correlate to a specific SO.
            worst_op = (
                per_rpc.get(dom_rpc, {}).get("worst_operator_id")
                if dom_rpc else None
            )
            dom_label = (
                f"{dom_rpc} (so-{worst_op})" if dom_rpc and worst_op is not None
                else (dom_rpc or "—")
            )
            rows.append([
                (t.get("payment_id") or "—")[:16],
                t.get("op") or "—",
                str(t.get("duration_ms") or "—"),
                fmt_ms(t.get("span_total_ms")),
                dom_label,
                fmt_ms(t.get("dominant_ms")),
                str(len(per_rpc)) if per_rpc else "—",
            ])
        out.extend(render_table(headers, rows))
        out.append("")
        # Per-RPC aggregate across the slow set (every individual call
        # of each RPC across every slow payment, regardless of operator).
        classified = agg.get("_classified", 0)
        out.append(
            f"#### Per-RPC aggregate over slow set "
            f"({classified} payments with phase data of {slow_count} slow)"
        )
        out.append("")
        agg_headers = ["rpc", "calls", "p50", "p95", "p99", "max", "% slow dominated by"]
        # Order RPCs by p99 descending — biggest contributors first.
        rpc_rows = []
        for rpc, stats in agg.items():
            if rpc.startswith("_"):
                continue
            rpc_rows.append((rpc, stats))
        rpc_rows.sort(key=lambda x: x[1].get("p99") or 0, reverse=True)
        agg_rows = []
        for rpc, stats in rpc_rows:
            dpct = stats.get("dominant_pct")
            agg_rows.append([
                rpc,
                str(stats.get("count", 0)),
                fmt_ms(stats.get("p50")),
                fmt_ms(stats.get("p95")),
                fmt_ms(stats.get("p99")),
                fmt_ms(stats.get("max")),
                "—" if dpct is None else f"{dpct:.0f}%",
            ])
        if agg_rows:
            out.extend(render_table(agg_headers, agg_rows))
            out.append("")
    return out


def render_results_md(sweep_id, manifest, steps_summary, headline, audit=None):
    # Unpack here so both the headline verdict (below) and the deeper
    # `max_safe_rps` line further down can consult the same values.
    max_safe, baseline_rps, baseline_p99, crossed_at = headline
    lines = []
    lines.append(f"# Bench RPS sweep — `{sweep_id}`")
    lines.append("")
    lines.append("## Sweep config")
    lines.append("")
    lines.append(f"- host: `{manifest.get('host', '?')}`")
    lines.append(f"- duration per step: `{manifest.get('duration_per_step', '?')}`")
    lines.append(f"- mix: `{manifest.get('mix', '?')}`")
    lines.append(f"- users: `{manifest.get('users', '?')}`  senders: `{manifest.get('senders', '?')}`")
    lines.append(f"- distribution: `{manifest.get('distribution', '?')}`")
    # Keep in sync with the HttpClient timeout in LoadGen.kt.
    lines.append(
        "- client request timeout: `60s` — a request slower than this is "
        "counted `timed_out` even if the server later completes it"
    )
    lines.append("")

    lines.append("## Headline")
    lines.append("")
    states = [(rps, step_state(s)) for rps, s in steps_summary]
    ok_rps = [r for r, st in states if st == "ok"]
    deg_rps = [r for r, st in states if st == "degrading"]
    col_rps = [r for r, st in states if st == "collapsed"]
    sustained = max(ok_rps) if ok_rps else None
    mix = manifest.get("mix", "?")

    # `step_state` is goodput-based: a step at congestion-collapse with
    # all dispatches eventually completing within the 60s drain window
    # still reads as "ok" even though p99 may have multiplied. So a
    # truthful headline must AND together (a) no degraded/collapsed
    # steps AND (b) no p99(send) cliff vs the swept range — the latter
    # is the `max_safe_rps < max swept RPS` signal from
    # `derive_p99_doubling`. Without (b), a queueing-only collapse
    # silently passes as "stable".
    all_rps = [r for r, _ in states]
    max_swept = max(all_rps) if all_rps else None
    latency_cliff = (
        max_safe is not None
        and max_swept is not None
        and max_safe < max_swept
    )

    if not deg_rps and not col_rps and not latency_cliff:
        verdict = (
            f"**Stable across the whole sweep** (mix `{mix}`): sustained "
            f"≥ {max_swept} RPS with no goodput or latency degradation observed."
        )
    elif not deg_rps and not col_rps and latency_cliff:
        # Goodput held but latency doubled — the cliff is queueing, not
        # error-collapse. Surface that explicitly: the partner sees an
        # n× p99 inflation while goodput still reads 100%.
        ratio = max(steps_summary, key=lambda x: x[0])[1] \
            .get("client_latency_ms", {}) \
            .get("_send_rollup", {}).get("p99")
        ratio_str = ""
        if ratio is not None and baseline_p99:
            ratio_str = f" — p99(send) inflated **{ratio / baseline_p99:.1f}×** vs baseline at top of sweep"
        verdict = (
            f"**Latency cliff** (mix `{mix}`): goodput held but p99(send) crossed 2× "
            f"baseline above **{max_safe} RPS** (baseline `{baseline_p99:.0f}`ms @ `{baseline_rps}` RPS, "
            f"crossed at `{crossed_at}` RPS){ratio_str}. The completion-rate ceiling is below the "
            f"dispatch rate at top of sweep; the in-flight queue grows monotonically — a real "
            f"external-dependency saturation."
        )
    else:
        head = (
            f"sustained ~**{sustained} RPS**"
            if sustained is not None
            else "**no step held ≥95% goodput**"
        )
        first_deg = min(deg_rps) if deg_rps else None
        tail = ""
        if col_rps:
            tail = f", congestion-**collapsed from {min(col_rps)} RPS**"
        elif first_deg is not None:
            tail = f", degrading from {first_deg} RPS (no full collapse in range)"
        caveat = (
            " Collapsed steps are **not reproducible measurements**: do not "
            "compare them or trust their latency percentiles (survivorship-biased; "
            "ordering is dominated by the external operator rate limit, closed-loop "
            "funding feedback, and per-step SIGKILL truncation)."
            if col_rps
            else ""
        )
        verdict = f"**Verdict** (mix `{mix}`): {head}{tail}.{caveat}"
    lines.append(f"- {verdict}")

    # Per-step state + goodput. Client = caller's view (60s timeout counts
    # against it); server = handlers actually completed, of everything offered.
    def _pct(num, den):
        return f"{(100.0 * num / den):.0f}%" if den else "—"

    state_str = "  ".join(f"{r}→{st}" for r, st in states)
    cli = "  ".join(
        f"{rps}→{_pct(s['client_ok'], s['total_dispatched'])}"
        for rps, s in steps_summary
    )
    srv = "  ".join(
        f"{rps}→{_pct(s['server_ok'], s['total_dispatched'])}"
        for rps, s in steps_summary
    )
    lines.append(f"- step state: {state_str}")
    lines.append(f"- client goodput (ok / offered): {cli}")
    lines.append(f"- server goodput (handler ok / offered): {srv}")

    if max_safe is not None:
        lines.append(
            f"- max_safe_rps (client p99(send) < 2× baseline, **pre-collapse "
            f"steps only**): `{max_safe}`, baseline `{baseline_p99:.0f}`ms @ "
            f"`{baseline_rps}` RPS"
        )
    lines.append("")

    # Op list is derived from data (union across steps), in canonical
    # order. New `_ln` variants render as their own rows alongside the
    # spark variants — distinct payment paths must not be blended in a
    # single percentile cell. The `_send_rollup`/`_receive_rollup`
    # synthetic entries are skipped here (they're only for the headline
    # cliff metric in derive_p99_doubling).
    _CANONICAL_OP_ORDER = [
        "info",
        "send", "send_spark", "send_ln",
        "receive", "receive_spark", "receive_ln",
    ]

    def _collect_ops(latency_key):
        seen = set()
        for _, s in steps_summary:
            for op in (s.get(latency_key) or {}).keys():
                if not op.startswith("_"):
                    seen.add(op)
        ordered = [op for op in _CANONICAL_OP_ORDER if op in seen]
        rest = sorted(seen - set(ordered))
        return ordered + rest

    lines.append(
        "> `state` ∈ ok / degrading / **collapsed**. Collapsed rows show "
        "`n/a` for latency on purpose — those percentiles are "
        "survivorship-biased and the rows are not comparable. Client counts "
        "are the caller's view (`ok+timed_out+failed+dropped = dispatched`; "
        "`timed_out` = past the 60s client timeout, server may have finished "
        "anyway); server counts are handlers actually logged "
        "(`completed = ok+err`, `≤ dispatched`)."
    )
    lines.append("")

    def render_lat_table(title, latency_key, count_headers, count_cells):
        ops = _collect_ops(latency_key)
        lines.append(title)
        lines.append("")
        headers = ["RPS", "state"] + count_headers
        for op in ops:
            headers += [f"{op} p50", f"{op} p95", f"{op} p99"]
        rows = []
        for rps, s in steps_summary:
            st = step_state(s)
            cells = [str(rps), st] + [str(c) for c in count_cells(s)]
            for op in ops:
                if st == "collapsed":
                    cells += ["n/a", "n/a", "n/a"]
                    continue
                stats = s[latency_key].get(op, {})
                cells.append(fmt_ms(stats.get("p50")))
                cells.append(fmt_ms(stats.get("p95")))
                cells.append(fmt_ms(stats.get("p99")))
            rows.append(cells)
        lines.extend(render_table(headers, rows))
        lines.append("")

    render_lat_table(
        "## Client-side latency (ms; client-successful requests only)",
        "client_latency_ms",
        ["dispatched", "dropped", "ok", "timed_out", "failed"],
        lambda s: [
            s["total_dispatched"],
            s["total_dropped"],
            s["client_ok"],
            s["client_timed_out"],
            s["client_failed"],
        ],
    )
    render_lat_table(
        "## Server-side latency (ms; handler-only, server-completed only)",
        "server_latency_ms",
        ["completed", "ok", "err"],
        lambda s: [s["server_completed"], s["server_ok"], s["server_err"]],
    )

    # Sub-timing breakdown: where does a send's latency go? `prepare_ms`
    # is one SSP RPC (fee estimate); `send_ms` is the heavy one (SSP
    # requestLightningSend + Spark transfer + storage). `build_ms` is
    # the per-request SDK build cost; near-zero means the shared
    # SdkContext is doing its job. Dominant column at the cliff = the
    # bottleneck row.
    def _has_send_subtimings():
        for _, s in steps_summary:
            sub = s.get("server_subtimings_ms") or {}
            for op in sub:
                if op.startswith("send"):
                    return True
        return False

    if _has_send_subtimings():
        lines.append("## Server-side send sub-timings (ms; successful sends only)")
        lines.append("")
        lines.append(
            "> `build` = SDK construction; `prepare` = `prepareSendPayment` "
            "(includes the SSP fee-estimate RPC for bolt11); `send` = "
            "`sendPayment` itself (SSP `requestLightningSend` + Spark "
            "transfer + storage); `disconnect` = SDK teardown. The dominant "
            "column where p99 inflates between RPS steps is the bottleneck."
        )
        lines.append("")
        send_ops = sorted({
            op for _, s in steps_summary
            for op in (s.get("server_subtimings_ms") or {}).keys()
            if op.startswith("send")
        })
        for op in send_ops:
            lines.append(f"### `{op}`")
            lines.append("")
            headers = ["RPS"]
            for phase in ("build", "prepare", "send", "disconnect"):
                headers += [f"{phase} p50", f"{phase} p95", f"{phase} p99"]
            rows = []
            for rps, s in steps_summary:
                cells = [str(rps)]
                opst = (s.get("server_subtimings_ms") or {}).get(op, {})
                for phase in ("build", "prepare", "send", "disconnect"):
                    stats = opst.get(f"{phase}_ms", {}) or {}
                    cells.append(fmt_ms(stats.get("p50")))
                    cells.append(fmt_ms(stats.get("p95")))
                    cells.append(fmt_ms(stats.get("p99")))
                rows.append(cells)
            lines.extend(render_table(headers, rows))
            lines.append("")

    lines.extend(render_swap_section(steps_summary))
    lines.extend(render_slow_payments_section(steps_summary))

    lines.append("## Process metrics")
    lines.append("")
    cpu_cores = next(
        (s["metrics"]["available_processors"] for _, s in steps_summary
         if s.get("metrics", {}).get("available_processors", -1) > 0),
        None,
    )
    if cpu_cores:
        lines.append(f"Host: {cpu_cores} logical CPU cores.")
        lines.append("")
    metrics_headers = [
        "RPS", "RSS mean (MiB)", "RSS max (MiB)", "heap used mean (MiB)",
        "process CPU mean", "process CPU max", "RPS / core",
        "mysql_conns max", "remote_tcp_sockets max", "fds max", "threads max",
    ]
    metrics_rows = []
    for rps, s in steps_summary:
        m = s.get("metrics", {})
        rss = m.get("rss_kb", {})
        heap = m.get("heap_used_bytes", {})
        mysql = m.get("mysql_conns", {})
        sock = m.get("remote_tcp_sockets", {})
        fds = m.get("fd_count", {})
        thr = m.get("thread_count", {})
        proc_cpu = m.get("process_cpu_load", {})
        cores = m.get("available_processors", -1)

        rps_per_core = None
        if cores > 0 and proc_cpu.get("mean") is not None and proc_cpu["mean"] > 0:
            cores_used = proc_cpu["mean"] * cores
            if cores_used > 0:
                rps_per_core = rps / cores_used

        metrics_rows.append([
            str(rps),
            fmt_kb_as_mib(rss.get("mean")),
            fmt_kb_as_mib(rss.get("max")),
            "—" if heap.get("mean") is None else f"{heap['mean'] / (1024*1024):.1f}",
            "—" if proc_cpu.get("mean") is None else f"{proc_cpu['mean'] * 100:.1f}%",
            "—" if proc_cpu.get("max") is None else f"{proc_cpu['max'] * 100:.1f}%",
            "—" if rps_per_core is None else f"{rps_per_core:.1f}",
            "—" if mysql.get("max") is None else str(int(mysql["max"])),
            "—" if sock.get("max") is None else str(int(sock["max"])),
            "—" if fds.get("max") is None else str(int(fds["max"])),
            "—" if thr.get("max") is None else str(int(thr["max"])),
        ])
    lines.extend(render_table(metrics_headers, metrics_rows))
    lines.append("")

    server_categories = sorted({
        cat
        for _, s in steps_summary
        for cat in s.get("errors_by_category_server", {})
    })
    if server_categories:
        lines.append("## Errors by category (server-side)")
        lines.append("")
        ec_headers = ["RPS", "total"] + list(server_categories)
        ec_rows = []
        for rps, s in steps_summary:
            cats = s.get("errors_by_category_server", {})
            total = sum(cats.values())
            row = [str(rps), str(total)]
            for cat in server_categories:
                row.append(str(cats.get(cat, 0)))
            ec_rows.append(row)
        lines.extend(render_table(ec_headers, ec_rows))
        lines.append("")

    # --- Lightning settlement audit ----------------------------------
    # Only emitted when audit.json is present. The audit re-syncs each
    # sender wallet and asks `listPayments(Send, Lightning)` what
    # actually settled on the LN side — `completionTimeoutSecs=0`
    # returns the moment the SSP accepts the payment, well before
    # settlement, so the per-step `client_ok` and `send_ln` p99 numbers
    # could otherwise be silently masking a tail of unsettled payments.
    # `expected` excludes both dropped and errored dispatches (no
    # sendPayment was ever called for those), so the denominator is
    # specifically "server-accepted send_ln dispatches".
    if audit:
        lines.append("## Lightning settlement audit")
        lines.append("")
        et = audit.get("expected_total", 0)
        c = audit.get("completed", 0)
        p = audit.get("pending", 0)
        f = audit.get("failed", 0)
        nf = audit.get("not_found", 0)
        sr = 100.0 * audit.get("settled_rate", 0.0)
        lines.append(
            f"- **{c}/{et} send_ln dispatches actually settled** "
            f"({sr:.1f}%). Pending={p}, Failed={f}, NotFound={nf}."
        )
        lines.append(
            "- `Completed` = SDK's listPayments observed `PaymentStatus.Completed` "
            "for the dispatched invoice on the sender wallet after a post-run "
            "syncWallet. `Pending` = SDK has the payment row but settlement "
            "hadn't propagated by audit time. `Failed` = SDK marked the payment "
            "Failed. `NotFound` = the server accepted the dispatch (2xx) but no "
            "Payment row matches the invoice (race / persistence bug). "
            "Dispatch-layer errors (HTTP 500, connection drops) are excluded "
            "from `expected`."
        )
        lines.append("")
        per_step = audit.get("per_step", [])
        if per_step:
            headers = ["RPS", "expected", "completed", "pending", "failed", "not_found", "settled %"]
            rows = []
            for s in per_step:
                exp = s.get("expected", 0)
                cc = s.get("completed", 0)
                pct = (100.0 * cc / exp) if exp else 0.0
                rows.append([
                    str(s.get("rps", "?")),
                    str(exp),
                    str(cc),
                    str(s.get("pending", 0)),
                    str(s.get("failed", 0)),
                    str(s.get("not_found", 0)),
                    f"{pct:.1f}",
                ])
            lines.extend(render_table(headers, rows))
            lines.append("")

    return "\n".join(lines)


# --- main ---------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description="Aggregate an RPS sweep")
    ap.add_argument("--sweep-dir", help="Path to out/<sweep-id>/")
    ap.add_argument(
        "--step-state",
        metavar="RPS_DIR",
        help="Classify one rps-<N> dir (ok|degrading|collapsed) to stdout "
        "and exit. Used by sweep.sh to stop the sweep at collapse.",
    )
    ap.add_argument(
        "--slow-abs-ms",
        type=int,
        default=2000,
        help="Absolute ms floor for the slow-payment section. A send is "
        "'slow' if its duration_ms ≥ max(this, step p<rel>).",
    )
    ap.add_argument(
        "--slow-rel-pct",
        type=int,
        default=95,
        help="Relative percentile used to bound the slow set per step "
        "(default p95). Combined with --slow-abs-ms via max().",
    )
    ap.add_argument(
        "--slow-top-n",
        type=int,
        default=20,
        help="How many slowest sends to list individually per step "
        "(default 20). Aggregate stats are over the entire slow set, "
        "not just the top-N.",
    )
    args = ap.parse_args()

    if args.step_state:
        s = aggregate_step(
            Path(args.step_state).resolve(),
            slow_abs_ms=args.slow_abs_ms,
            slow_rel_pct=args.slow_rel_pct,
            slow_top_n=args.slow_top_n,
        )
        # No data ⇒ treat as collapsed so the sweep stops rather than
        # marching on into more junk steps.
        print(step_state(s) if s is not None else "collapsed")
        return

    if not args.sweep_dir:
        ap.error("--sweep-dir is required (or use --step-state)")

    sweep_dir = Path(args.sweep_dir).resolve()
    if not sweep_dir.is_dir():
        print(f"error: sweep dir not found: {sweep_dir}", file=sys.stderr)
        sys.exit(2)

    manifest_path = sweep_dir / "manifest.json"
    manifest = {}
    if manifest_path.exists():
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except json.JSONDecodeError as e:
            print(f"warn: malformed manifest.json: {e}", file=sys.stderr)

    sweep_id = manifest.get("sweep_id") or sweep_dir.name

    steps_summary = []
    for rps, step_dir in step_dirs(sweep_dir):
        s = aggregate_step(
            step_dir,
            slow_abs_ms=args.slow_abs_ms,
            slow_rel_pct=args.slow_rel_pct,
            slow_top_n=args.slow_top_n,
        )
        if s is None:
            print(f"warn: skipping {step_dir.name} (no data)", file=sys.stderr)
            continue
        steps_summary.append((rps, s))

    if not steps_summary:
        print("error: no steps to aggregate", file=sys.stderr)
        sys.exit(2)

    headline = derive_p99_doubling(steps_summary)
    max_safe, baseline_rps, baseline_p99, crossed_at = headline

    summary = {
        "sweep_id": sweep_id,
        "manifest": manifest,
        "headline": {
            "max_safe_rps": max_safe,
            "p99_baseline_rps": baseline_rps,
            "p99_baseline_ms": baseline_p99,
            "p99_doubling_rps": crossed_at,
        },
        "steps": [{"rps": rps, **s} for rps, s in steps_summary],
    }
    summary_path = sweep_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")

    audit = None
    audit_path = sweep_dir / "audit.json"
    if audit_path.is_file():
        try:
            audit = json.loads(audit_path.read_text(encoding="utf-8"))
        except Exception as e:
            print(f"warn: malformed audit.json: {e}", file=sys.stderr)
    md = render_results_md(sweep_id, manifest, steps_summary, headline, audit)
    results_path = sweep_dir / "RESULTS.md"
    results_path.write_text(md, encoding="utf-8")

    print(f"[aggregate] wrote {summary_path}")
    print(f"[aggregate] wrote {results_path}")
    if max_safe is None:
        print("[aggregate] headline: insufficient send data")
    else:
        print(
            f"[aggregate] headline: max_safe_rps={max_safe} "
            f"(baseline p99={baseline_p99:.1f}ms @ {baseline_rps} RPS, "
            f"doubling@{crossed_at})"
        )


if __name__ == "__main__":
    main()
