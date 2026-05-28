#!/usr/bin/env python3
"""Compute workload-sized fund + seed-senders + invoice-pool budget.

Reads sweep config from CLI args, prints three ints on stdout:
`per_sender_sats treasurer_target_sats invoice_count`. Stdlib only.

`invoice_count` is 0 for spark-only mixes (no `send_ln` in --mix).
The two-int legacy contract is dropped in favor of always printing three
so the sweep driver doesn't have to branch.
"""

import argparse
import math
from pathlib import Path


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
    ap.add_argument("--mix", required=True,
                    help="Op mix; labels: info, send (alias send_spark), send_ln, "
                         "receive (alias receive_spark), receive_ln")
    ap.add_argument("--senders", type=int, required=True)
    ap.add_argument("--payment-sats", type=int, required=True)
    ap.add_argument("--safety", type=float, default=2.0,
                    help="Per-sender safety multiplier on drain")
    ap.add_argument("--buffer", type=float, default=1.5,
                    help="Treasurer buffer over K × per_sender")
    ap.add_argument("--floor", type=int, default=50_000,
                    help="Minimum treasurer target")
    ap.add_argument("--ln-fee-sats", type=int, default=None,
                    help="Per-send LN fee headroom for send_ln drain. "
                         "Falls back to --ln-fee-file when omitted, else 0. "
                         "Caller is responsible for any safety multiplier; "
                         "this value is added verbatim to per-send drain.")
    ap.add_argument("--ln-fee-file", default=None,
                    help="Sidecar file written by mint-invoices (<pool>.fee), "
                         "containing the raw SSP-probed fee. Used as a fallback "
                         "for --ln-fee-sats so manual reruns don't have to "
                         "re-pass the probed value.")
    ap.add_argument("--invoice-safety", type=float, default=1.05,
                    help="Pool-count safety multiplier for send_ln dispatches; separate "
                         "from --safety. Pool overshoot is cheap (1-2 extra dispatches "
                         "from dispatch-loop rounding) so 1.05 absorbs it without 2x'ing "
                         "the SSP-bound pre-mint time")
    args = ap.parse_args()

    rps_list = [int(x.strip()) for x in args.rps.split(",") if x.strip()]
    dur_s = parse_duration_secs(args.duration)
    mix = parse_mix(args.mix)
    total_w = sum(mix.values())

    if args.ln_fee_sats is not None:
        ln_fee_sats = args.ln_fee_sats
    elif args.ln_fee_file and Path(args.ln_fee_file).is_file():
        ln_fee_sats = int(Path(args.ln_fee_file).read_text().strip())
    else:
        ln_fee_sats = 0

    # Bare `send`/`receive` are spark aliases (back-compat). LN variants
    # are their own labels; their drain includes the SSP fee headroom.
    spark_send_w = mix.get("send", 0.0) + mix.get("send_spark", 0.0)
    ln_send_w = mix.get("send_ln", 0.0) + mix.get("send_lightning", 0.0) + mix.get("send_bolt11", 0.0)
    spark_send_frac = (spark_send_w / total_w) if total_w > 0 else 0.0
    ln_send_frac = (ln_send_w / total_w) if total_w > 0 else 0.0

    total_load = sum(rps * dur_s for rps in rps_list)
    total_spark_sends = total_load * spark_send_frac
    total_ln_sends = total_load * ln_send_frac

    if args.senders <= 0:
        per_sender = 0
    else:
        per_sender_drain = (
            total_spark_sends * args.payment_sats
            + total_ln_sends * (args.payment_sats + ln_fee_sats)
        ) / args.senders
        per_sender = math.ceil(per_sender_drain * args.safety)
    treasurer = max(int(args.senders * per_sender * args.buffer), args.floor)
    invoice_count = math.ceil(total_ln_sends * args.invoice_safety)
    print(per_sender, treasurer, invoice_count)


if __name__ == "__main__":
    main()
