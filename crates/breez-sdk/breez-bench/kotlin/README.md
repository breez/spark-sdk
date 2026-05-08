# Breez SDK server-side bench

HTTP server that wraps the Breez SDK (one fresh SDK instance per
request, against a shared MySQL backend) plus an open-loop load
generator. Drives a sweep of target RPS values, samples server-side
metrics + per-request latency, and emits a tabular `RESULTS.md`. Used
to estimate per-pod capacity for partners running the SDK server-side
at multi-tenant scale.

Design rationale and the longer "why" live in `benchmark-plan.md` at
the repo root; this file is just how to run it.

## Quickstart

```bash
make mysql-up           # idempotent: starts container + creates DB
make setup              # one-time per branch (publishes KMP bindings)
export FAUCET_USERNAME=...     # request from Lightspark
export FAUCET_PASSWORD=...
make run                # sweep + auto-fund + aggregate (~7 min default)
# → out/<sweep-id>/RESULTS.md
```

All three steps are idempotent. Subsequent sessions just need
`make run` since the closed-loop funding flow keeps the bench wallets
populated.

## Output

```
out/<sweep-id>/
  manifest.json            sweep config + host info
  RESULTS.md               headline tables (read this)
  summary.json             full structured per-step breakdown
  fund.log seed.log        pre-sweep steps (skipped if no-op)
  rps-50/
    server.log             server stdout/stderr
    loadgen.log            loadgen stdout/stderr
    requests.jsonl         server-side per-request timings
    metrics.jsonl          1Hz process metrics
    latency.jsonl          client-side per-request timings
  rps-100/  ...
```

`out/` is gitignored. Headline numbers live wherever they're shared
(Slack, partner doc, etc.), not in git.

## Tweaking the sweep

All knobs are env vars. Defaults give the fast pass.

| Var | Default | Notes |
|---|---|---|
| `SWEEP_RPS` | `50,100,250` | Comma-separated RPS list |
| `DURATION` | `2m` | Per-step duration (`Xs` / `Xm` / `Xh`) |
| `WARMUP_SECS` | `60` | First N seconds of each step excluded from stats |
| `MIX` | `info=40,receive=30,send=30` | Op weights (any positive numbers) |
| `USERS` | `10000` | Workload pool size for `/info` + `/receive` |
| `SENDERS` | `50` | Sender wallet pool for `/send` |
| `DIST` | `uniform` | `uniform` or `zipf` |
| `PAYMENT_SATS` | `1` | Sats per `/send` |
| `PORT` | `8080` | HTTP listen port |
| `MASTER_SECRET` | `breez-bench` | Wallet seed namespace; standardized so wallets persist between runs |
| `MYSQL_URL` | `mysql://root:password@127.0.0.1:3306/breez_bench` | Bench DB |
| `SWEEP_ID` | fresh ISO-8601 timestamp | Set explicitly to share a directory across phases |

Headline-grade run: `SWEEP_RPS=50,100,250,500,1000 DURATION=5m make run`
(~30 min wall time).

## Other commands

- `make smoke-test` — single SDK `connect → getInfo → disconnect`.
  Sanity check that the binding + MySQL path work.
- `make run-server` — server only. Useful for `curl`-driven probes.
- `make loadgen TARGET_RPS=100 DURATION=5m` — load generator only,
  against an already-running server.
- `make sweep` — sweep only, no aggregation.
- `make aggregate SWEEP_ID=…` — aggregate a sweep dir into
  `summary.json` + `RESULTS.md`.
- `make fund TARGET_SATS=N` — top up the treasurer wallet to N sats.
  Auto-run by `make run` with a workload-derived value; this is for
  ad-hoc / debugging.
- `make seed-senders PER_SENDER_SATS=N` — same, for the K sender
  wallets.
- `make help` — list all targets.

## Endpoints

The server exposes three:

| Endpoint | Body | Maps to |
|---|---|---|
| `GET /users/{userId}/info` | — | `getInfo({ ensureSynced: true })` |
| `POST /users/{userId}/send` | `{"destination":"<spark addr>","amountSats":N}` | `prepareSendPayment` + `sendPayment` |
| `POST /users/{userId}/receive` | `{}` | `receivePayment(SparkAddress)` (address generation only) |

Each request does a fresh `connect → op → disconnect` against the
shared MySQL backend. Same-`userId` requests serialize on a per-user
mutex; different user-ids run in parallel.
