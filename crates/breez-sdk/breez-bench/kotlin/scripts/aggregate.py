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


# --- per-step aggregation ------------------------------------------------

def per_op_latency(rows, duration_field, op_field):
    """Group durations by op, return summary stats per op."""
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
    return {op: summary_stats(v) for op, v in by_op.items()}


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


def aggregate_step(step_dir):
    """Aggregate one rps-N directory. Returns a dict (or None if no data)."""
    latency_rows = read_jsonl(step_dir / "latency.jsonl")
    requests_rows = read_jsonl(step_dir / "requests.jsonl")
    metrics_rows = read_jsonl(step_dir / "metrics.jsonl")

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

    metrics = metrics_window(metrics_rows, ts_lo, ts_hi) if metrics_rows else {}

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
        "metrics": metrics,
        "errors_by_category_client": client_errors_by_category,
        "errors_by_category_server": server_errors_by_category,
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
        send = s.get("client_latency_ms", {}).get("send")
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


def render_results_md(sweep_id, manifest, steps_summary, headline):
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

    if not deg_rps and not col_rps:
        all_rps = [r for r, _ in states]
        verdict = (
            f"**Stable across the whole sweep** (mix `{mix}`): sustained "
            f"≥ {max(all_rps)} RPS with no degradation observed."
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

    max_safe, baseline_rps, baseline_p99, crossed_at = headline
    if max_safe is not None:
        lines.append(
            f"- max_safe_rps (client p99(send) < 2× baseline, **pre-collapse "
            f"steps only**): `{max_safe}`, baseline `{baseline_p99:.0f}`ms @ "
            f"`{baseline_rps}` RPS"
        )
    lines.append("")

    ops = ["info", "send", "receive"]

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
    args = ap.parse_args()

    if args.step_state:
        s = aggregate_step(Path(args.step_state).resolve())
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
        s = aggregate_step(step_dir)
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

    md = render_results_md(sweep_id, manifest, steps_summary, headline)
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
