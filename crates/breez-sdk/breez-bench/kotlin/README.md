# Breez SDK server-side bench

HTTP server that wraps the Breez SDK (one fresh SDK instance per
request, against a shared MySQL backend) plus an open-loop load
generator. Drives a sweep of target RPS values, samples server-side
metrics + per-request latency, and emits a tabular `RESULTS.md`. Used
to estimate per-pod capacity for partners running the SDK server-side
at multi-tenant scale.

Design rationale and the longer "why" live in `DESIGN.md` next to
this file; this file is just how to run it.

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

One sweep → one directory `out/<sweep-id>/` (gitignored). Default
`<sweep-id>` is a UTC ISO-8601 timestamp; `LABEL=<kebab-slug> make run`
appends a human suffix (e.g. `2026-05-20T11-45-00Z-mysql-recycle`).

```
out/<sweep-id>/
  manifest.json            sweep config + host info + funding budget + MASTER_SECRET
  driver.log               live [sweep]/[fund]/[seed]/[loadgen] output
  RESULTS.md               headline tables (read this)
  summary.json             full structured per-step breakdown
  fund.log seed.log        pre-sweep step output (skipped if no-op)
  rps-50/
    server.log loadgen.log
    requests.jsonl         server-side per-request timings
    metrics.jsonl          1Hz process metrics
    latency.jsonl          client-side per-request timings
  rps-100/  ...
```

## Tweaking the sweep

All knobs are env vars. Defaults give the fast pass.

| Var | Default | Notes |
|---|---|---|
| `SWEEP_RPS` | `50,100,250` | Comma-separated RPS list |
| `DURATION` | `2m` | Per-step duration (`Xs` / `Xm` / `Xh`) |
| `MIX` | `info=40,receive=30,send=30` | Op weights (`label=N,...`). Labels: `info`, `send`/`send_spark`, `send_ln`, `receive`/`receive_spark`, `receive_ln`. Bare `send`/`receive` = spark variants |
| `USERS` | `10000` | Workload pool size for `/info` + `/receive` |
| `SENDERS` | `50` | Sender wallet pool for `/send` |
| `BANKS` | `50` | Receiver wallets for the bolt11 pre-mint pool (only used when `MIX` has `send_ln`) |
| `INVOICE_EXPIRY_SECS` | `604800` | Expiry per pre-minted invoice (7 days) |
| `MINT_PARALLELISM` | `20` | Concurrent in-flight `receivePayment` during pre-mint |
| `LN_FEE_SAFETY` | `1.5` | Headroom multiplier on the probed LN fee for sender prefunding |
| `DIST` | `uniform` | `uniform` or `zipf` |
| `PAYMENT_SATS` | `1` | Sats per `/send` |
| `PORT` | `8080` | HTTP listen port |
| `MASTER_SECRET` | `breez-bench` | Wallet seed namespace; standardized so wallets persist between runs |
| `MYSQL_URL` | `mysql://root:password@127.0.0.1:3306/breez_bench` | Bench DB |
| `CONNS_PER_OPERATOR` | unset → `null` | gRPC conns per operator. `null` = one multiplexed (prod default); set int to fan out for capacity-isolation runs |
| `MYSQL_MAX_POOL` | unset → SDK default | Max conns in the shared MySQL pool. Must stay below the server's `max_connections` (see Makefile) |
| `LOG_FILTER` | unset → off | Rust SDK tracing filter (`tracing` `EnvFilter` syntax). Logs to `out/<id>/rps-N/.trace-logs/sdk.log` |
| `LABEL` | unset | Optional kebab-case slug appended to `SWEEP_ID` (`<timestamp>-<label>`) |
| `SWEEP_ID` | fresh UTC ISO-8601 timestamp | Override only to resume an existing sweep dir |

Headline-grade run: `SWEEP_RPS=50,100,250,500,1000 DURATION=5m make run`
(~30 min wall time).

## Op-mix vocabulary

The `MIX` weights are the spark-vs-Lightning percentage control — labels
name the payment method. Bare `send`/`receive` mean the spark variants.

| Label | Endpoint | Behavior |
|---|---|---|
| `info` | `GET /info` | Local read, no sync. |
| `send` (alias `send_spark`) | `POST /send` | Spark transfer → treasurer Spark addr. Zero-fee, closed-loop. |
| `send_ln` | `POST /send` | Pay a pre-minted bolt11 invoice via real LN (SSP-routed). Non-zero fee. Requires the pre-mint step. |
| `receive` (alias `receive_spark`) | `POST /receive` | Spark address generation (no SSP roundtrip). |
| `receive_ln` | `POST /receive` | Mint a real bolt11 invoice on the user's wallet (one SSP roundtrip). |

Example mixed run:

```bash
MIX='info=40,send_spark=20,send_ln=10,receive_spark=20,receive_ln=10' \
  SWEEP_RPS=50,100,250 DURATION=2m make run
```

### Lightning specifics

Adding `send_ln` to `MIX` triggers a one-shot pre-mint step (sized from
the sweep config) before the RPS loop. **Pool exhaustion is a hard-stop**
(non-zero exit) — re-run and the sweep driver re-mints automatically.

Real LN fees are non-zero, so the closed-loop sat-conservation breaks;
net leakage is replenished from the faucet on the next `make fund`.

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
- `make mint-invoices COUNT=N OUT=path` — pre-mint a pool of bolt11
  invoices (used by sweeps with `send_ln` in the mix). Auto-run by the
  sweep driver with workload-sized `COUNT`; this is for ad-hoc /
  debugging.
- `make help` — list all targets.

## Endpoints

| Endpoint | Body | Maps to |
|---|---|---|
| `GET /users/{userId}/info` | — | `getInfo({ ensureSynced: false })` (local read, no sync) |
| `POST /users/{userId}/send` | `{"destination":"<spark addr or bolt11>","amountSats":N}` | `prepareSendPayment` + `sendPayment`; LN options injected when `prepared.paymentMethod` is `Bolt11Invoice` |
| `POST /users/{userId}/receive` | `{}` or `{"method":"bolt11","amountSats":N}` | `receivePayment(SparkAddress)` (default) or `receivePayment(Bolt11Invoice)` (real LN, SSP-mediated) |
