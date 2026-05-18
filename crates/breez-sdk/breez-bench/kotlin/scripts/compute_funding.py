#!/usr/bin/env python3
"""Compute workload-sized fund + seed-senders budget for a sweep.

Reads sweep config from CLI args, prints two integers on stdout:
`per_sender_sats treasurer_target_sats`. Stdlib only.
"""

import argparse
import math


def parse_duration_secs(s: str) -> int:
    s = s.lower().strip()
    if s.endswith("h"):  return int(s[:-1]) * 3600
    if s.endswith("m"): return int(s[:-1]) * 60
    if s.endswith("s"): return int(s[:-1])
    return int(s)


def parse_mix(spec: str) -> dict[str, float]:
    out: dict[str, float] = {}
    for entry in spec.split(","):
        k, v = entry.split("=", 1)
        out[k.strip()] = float(v.strip())
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--rps", required=True, help="Comma-separated RPS list")
    ap.add_argument("--duration", required=True, help="Per-step duration (e.g. 5m, 30s)")
    ap.add_argument("--mix", required=True, help="Op mix, e.g. info=40,receive=30,send=30")
    ap.add_argument("--senders", type=int, required=True)
    ap.add_argument("--payment-sats", type=int, required=True)
    ap.add_argument("--safety", type=float, default=2.0)
    ap.add_argument("--buffer", type=float, default=1.5)
    ap.add_argument("--floor", type=int, default=50_000)
    args = ap.parse_args()

    rps_list = [int(x.strip()) for x in args.rps.split(",") if x.strip()]
    dur_s = parse_duration_secs(args.duration)
    mix = parse_mix(args.mix)
    total_w = sum(mix.values())
    send_frac = (mix.get("send", 0.0) / total_w) if total_w > 0 else 0.0

    total_sends = sum(rps * dur_s for rps in rps_list) * send_frac
    sends_per_sender = total_sends / args.senders if args.senders else 0
    per_sender = math.ceil(sends_per_sender * args.payment_sats * args.safety)
    treasurer = max(int(args.senders * per_sender * args.buffer), args.floor)
    print(per_sender, treasurer)


if __name__ == "__main__":
    main()
