#!/usr/bin/env python3
"""Aggregate a Phase 6 RPS sweep into the headline 3 numbers.

Walks `out/<sweep-id>/rps-<N>/` directories, reads `latency.jsonl`
(client-side observed timings + warmup flag), `requests.jsonl`
(server-side handler timings, no warmup flag — derived), and
`metrics.jsonl` (1Hz process samples). Writes `summary.json` and
`RESULTS.md` to the sweep dir.

Stdlib only — no numpy / pandas / matplotlib. Phase 9 will add
matplotlib for charts.
"""

import argparse
import json
import math
import sys
from pathlib import Path


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

def per_op_latency(rows, duration_field, op_field, *, post_warmup_only=False, warmup_cutoff_ts=None):
    """Group durations by op, return summary stats per op."""
    by_op = {}
    for r in rows:
        if post_warmup_only and warmup_cutoff_ts is not None and r.get("ts", 0) < warmup_cutoff_ts:
            continue
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


def warmup_cutoff(latency_rows):
    """Return the smallest ts where warmup=false (i.e., post-warmup window
    start). If no such row exists, return +inf so everything is filtered out."""
    cutoffs = [r["ts"] for r in latency_rows if r.get("warmup") is False and "ts" in r]
    if not cutoffs:
        return float("inf")
    return min(cutoffs)


def metrics_window(metrics_rows, ts_lo, ts_hi):
    """Slice metrics samples to [ts_lo, ts_hi]; -1 sentinels are filtered."""
    rss, heap_used, mysql, sockets, fds, threads = [], [], [], [], [], []
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
        ):
            v = m.get(src)
            if v is not None and v >= 0:
                dst.append(v)
    return {
        "rss_kb": summary_stats(rss),
        "heap_used_bytes": summary_stats(heap_used),
        "mysql_conns": summary_stats(mysql),
        "remote_tcp_sockets": summary_stats(sockets),
        "fd_count": summary_stats(fds),
        "thread_count": summary_stats(threads),
    }


def aggregate_step(step_dir):
    """Aggregate one rps-N directory. Returns a dict (or None if no data)."""
    latency_rows = read_jsonl(step_dir / "latency.jsonl")
    requests_rows = read_jsonl(step_dir / "requests.jsonl")
    metrics_rows = read_jsonl(step_dir / "metrics.jsonl")

    if not latency_rows and not requests_rows:
        return None

    warm_ts = warmup_cutoff(latency_rows)

    # Step time bounds: from earliest non-warmup latency row to last entry seen.
    if warm_ts == float("inf"):
        ts_lo = min((r["ts"] for r in latency_rows if "ts" in r), default=0)
    else:
        ts_lo = int(warm_ts)
    all_ts = (
        [r["ts"] for r in latency_rows if "ts" in r]
        + [r["ts"] for r in requests_rows if "ts" in r]
    )
    ts_hi = max(all_ts) if all_ts else ts_lo

    # --- counts (whole step, not just post-warmup) for visibility -------
    total_dispatched = sum(1 for r in latency_rows)
    total_dropped = sum(1 for r in latency_rows if r.get("dropped"))
    total_errors = sum(
        1 for r in latency_rows if r.get("error") is not None and not r.get("dropped")
    )

    # --- post-warmup latency per op (client + server view) --------------
    client_lat = per_op_latency(
        latency_rows,
        duration_field="duration_ms",
        op_field="op",
        post_warmup_only=True,
        warmup_cutoff_ts=warm_ts if warm_ts != float("inf") else None,
    )
    server_lat = per_op_latency(
        requests_rows,
        duration_field="duration_ms",
        op_field="op",
        post_warmup_only=True,
        warmup_cutoff_ts=warm_ts if warm_ts != float("inf") else None,
    )

    # --- post-warmup metrics window -------------------------------------
    metrics = metrics_window(metrics_rows, ts_lo, ts_hi) if metrics_rows else {}

    return {
        "ts_window_ms": [ts_lo, ts_hi],
        "total_dispatched": total_dispatched,
        "total_dropped": total_dropped,
        "total_errors_post_dispatch": total_errors,
        "client_latency_ms": client_lat,
        "server_latency_ms": server_lat,
        "metrics": metrics,
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


# --- markdown rendering --------------------------------------------------

def fmt_ms(v):
    return "—" if v is None else f"{v:.1f}"


def fmt_kb_as_mib(v):
    if v is None:
        return "—"
    return f"{v / 1024.0:.1f}"


def render_results_md(sweep_id, manifest, steps_summary, headline):
    lines = []
    lines.append(f"# Bench RPS sweep — `{sweep_id}`")
    lines.append("")
    lines.append("**Worst-case (Phase 6) baseline.** Per-request SDK lifecycle, no")
    lines.append("pooling, no shared MySQL pool, no shared operator pool. Two SDK-")
    lines.append("level optimisations in flight (shared MySQL pool, shared operator")
    lines.append("pool) and Phase 7 (LRU SDK-instance pool) push these numbers")
    lines.append("favourably. Treat as upper bound on latency / lower bound on")
    lines.append("capacity.")
    lines.append("")
    lines.append("## Sweep config")
    lines.append("")
    lines.append(f"- host: `{manifest.get('host', '?')}`")
    lines.append(f"- duration per step: `{manifest.get('duration_per_step', '?')}`")
    lines.append(f"- warmup: `{manifest.get('warmup_secs', '?')}s`")
    lines.append(f"- mix: `{manifest.get('mix', '?')}`")
    lines.append(f"- users: `{manifest.get('users', '?')}`  senders: `{manifest.get('senders', '?')}`")
    lines.append(f"- distribution: `{manifest.get('distribution', '?')}`")
    lines.append("")

    lines.append("## Headline 3")
    lines.append("")
    max_safe, baseline_rps, baseline_p99, crossed_at = headline
    if max_safe is None:
        lines.append("- **Max RPS before p99(send) doubles**: insufficient `send` data — check funding / errors.")
    else:
        baseline_str = f"{baseline_p99:.1f} ms @ {baseline_rps} RPS"
        lines.append(f"- **Max RPS before p99(send) doubles**: `{max_safe}` RPS (baseline `{baseline_str}`)")
        if crossed_at is not None:
            lines.append(f"  - Threshold crossed at `{crossed_at}` RPS.")
        else:
            lines.append(f"  - Threshold not crossed within sweep range — sweep should extend higher.")
    lines.append("- **Per-op latency** at each RPS: see *Client-side latency* table below.")
    lines.append("- **Memory at sustained RPS**: see *Process metrics* table below.")
    lines.append("")

    lines.append("## Client-side latency (ms; post-warmup; successful requests only)")
    lines.append("")
    ops = ["info", "send", "receive"]
    header = "| RPS | dispatched | dropped | errors | " + " | ".join(
        f"{op} p50 | {op} p95 | {op} p99" for op in ops
    ) + " |"
    sep = "|" + "---|" * (4 + 3 * len(ops))
    lines.append(header)
    lines.append(sep)
    for rps, s in steps_summary:
        cells = [str(rps), str(s["total_dispatched"]), str(s["total_dropped"]), str(s["total_errors_post_dispatch"])]
        for op in ops:
            stats = s["client_latency_ms"].get(op, {})
            cells.append(fmt_ms(stats.get("p50")))
            cells.append(fmt_ms(stats.get("p95")))
            cells.append(fmt_ms(stats.get("p99")))
        lines.append("| " + " | ".join(cells) + " |")
    lines.append("")

    lines.append("## Server-side latency (ms; handler-only; post-warmup)")
    lines.append("")
    lines.append("Server handler time excludes network and any pre-handler queueing")
    lines.append("at Netty. Gap vs. client-side latency is queue + transport.")
    lines.append("")
    lines.append(header)
    lines.append(sep)
    for rps, s in steps_summary:
        cells = [str(rps), str(s["total_dispatched"]), str(s["total_dropped"]), str(s["total_errors_post_dispatch"])]
        for op in ops:
            stats = s["server_latency_ms"].get(op, {})
            cells.append(fmt_ms(stats.get("p50")))
            cells.append(fmt_ms(stats.get("p95")))
            cells.append(fmt_ms(stats.get("p99")))
        lines.append("| " + " | ".join(cells) + " |")
    lines.append("")

    lines.append("## Process metrics (post-warmup window)")
    lines.append("")
    lines.append("| RPS | RSS mean (MiB) | RSS max (MiB) | heap used mean (MiB) | mysql_conns max | remote_tcp_sockets max | fds max | threads max |")
    lines.append("|---|---|---|---|---|---|---|---|")
    for rps, s in steps_summary:
        m = s.get("metrics", {})
        rss = m.get("rss_kb", {})
        heap = m.get("heap_used_bytes", {})
        mysql = m.get("mysql_conns", {})
        sock = m.get("remote_tcp_sockets", {})
        fds = m.get("fd_count", {})
        thr = m.get("thread_count", {})
        cells = [
            str(rps),
            fmt_kb_as_mib(rss.get("mean")),
            fmt_kb_as_mib(rss.get("max")),
            "—" if heap.get("mean") is None else f"{heap['mean'] / (1024*1024):.1f}",
            "—" if mysql.get("max") is None else str(int(mysql["max"])),
            "—" if sock.get("max") is None else str(int(sock["max"])),
            "—" if fds.get("max") is None else str(int(fds["max"])),
            "—" if thr.get("max") is None else str(int(thr["max"])),
        ]
        lines.append("| " + " | ".join(cells) + " |")
    lines.append("")

    lines.append("## Framing")
    lines.append("")
    lines.append("If the latency cliff coincides with local resource saturation")
    lines.append("(RSS climbing fast, FDs near limit, mysql_conns saturated, CPU pegged)")
    lines.append("the headline is **SDK-bounded**. If local resources stay idle")
    lines.append("through the cliff, the headline is **regtest-bounded** — the")
    lines.append("Lightspark public regtest operators are throttling. Real partner")
    lines.append("deployments on dedicated infrastructure should expect higher")
    lines.append("ceilings.")
    lines.append("")
    return "\n".join(lines)


# --- main ---------------------------------------------------------------

def main():
    ap = argparse.ArgumentParser(description="Aggregate a Phase 6 RPS sweep")
    ap.add_argument("--sweep-dir", required=True, help="Path to out/<sweep-id>/")
    args = ap.parse_args()

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
