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

## Output naming — read before kicking off a sweep

**One sweep → one directory `out/<sweep-id>/`.** Naming rule (enforced by `sweep.sh`, warn-on-deviate):

```
out/<sweep-id>/    where  <sweep-id> = <UTC-ISO-8601-timestamp>[-<label>]
```

Examples:

```
out/2026-05-20T11-45-00Z/                       # default: just the timestamp
out/2026-05-20T11-45-00Z-mysql-recycle/         # LABEL=mysql-recycle make run
out/2026-05-20T11-45-00Z-phase6-fastpass/       # LABEL=phase6-fastpass make run
```

Rules:

- **Always** start with the UTC timestamp prefix so `ls out/` sorts chronologically and entries from different sessions don't collide.
- Tag a run with `LABEL=<kebab-slug>` (`[a-z0-9][a-z0-9-]*`). Don't bake the label into a custom `SWEEP_ID` — let the Makefile compose it from `LABEL`.
- **No bare files at the top level of `out/`.** Everything a run produces — including the live driver log — lives inside the sweep dir. Don't `make run > out/foo.log 2>&1`; the recipe already tees its combined stdout/stderr into `out/<sweep-id>/driver.log` for you (so progress is visible on the terminal AND persisted).
- Set `SWEEP_ID=…` directly only to resume / share an existing dir across phases. `sweep.sh` warns if `SWEEP_ID` doesn't match the convention but still runs.

**Inside the sweep dir:**

```
out/<sweep-id>/
  manifest.json            sweep config + host info + funding budget
  driver.log               live [sweep]/[fund]/[seed]/[loadgen] output (tee'd by `make run`)
  RESULTS.md               headline tables (read this)
  summary.json             full structured per-step breakdown
  fund.log seed.log        pre-sweep step output (skipped if no-op)
  rps-50/
    server.log             server stdout/stderr
    loadgen.log            loadgen stdout/stderr
    requests.jsonl         server-side per-request timings
    metrics.jsonl          1Hz process metrics
    latency.jsonl          client-side per-request timings
  rps-100/  ...
```

`out/` is gitignored. Headline numbers live wherever they're shared (Slack, partner doc, etc.), not in git.

## Tweaking the sweep

All knobs are env vars. Defaults give the fast pass.

| Var | Default | Notes |
|---|---|---|
| `SWEEP_RPS` | `50,100,250` | Comma-separated RPS list |
| `DURATION` | `2m` | Per-step duration (`Xs` / `Xm` / `Xh`) |
| `MIX` | `info=40,receive=30,send=30` | Op weights (any positive numbers) |
| `USERS` | `10000` | Workload pool size for `/info` + `/receive` |
| `SENDERS` | `50` | Sender wallet pool for `/send` |
| `DIST` | `uniform` | `uniform` or `zipf` |
| `PAYMENT_SATS` | `1` | Sats per `/send` |
| `PORT` | `8080` | HTTP listen port |
| `MASTER_SECRET` | `breez-bench` | Wallet seed namespace; standardized so wallets persist between runs |
| `MYSQL_URL` | `mysql://root:password@127.0.0.1:3306/breez_bench` | Bench DB |
| `CONNS_PER_OPERATOR` | unset → `null` | gRPC connections per operator in the shared context. `null` = one multiplexed connection (prod default); set a positive int to fan out and check whether that single connection caps top-of-sweep throughput |
| `MYSQL_MAX_POOL` | unset → SDK default (`num_cpus*4`) | Max connections in the shared MySQL pool. Set a positive int to probe whether the default pool caps top-of-sweep throughput; compare against `mysql_conns` in `metrics.jsonl`. **Must stay below the server's `max_connections`** (set on container start by `make mysql-up`; see `MYSQL_SERVER_MAX_CONNECTIONS` in the Makefile) — otherwise the server rejects new connections with `HY000: Too many connections` |
| `LOG_FILTER` | unset → off | Rust SDK tracing in the per-step server (tracing `EnvFilter`). e.g. `warn,spark_wallet=info,spark::operator::rpc=debug` logs every operator gRPC method call + rate-limit retries to `out/<id>/rps-N/.trace-logs/sdk.log`. Off by default (no overhead) |
| `LABEL` | unset | Optional kebab-case slug appended to the auto-generated `SWEEP_ID`. Use this rather than a custom `SWEEP_ID` so runs stay sortable. See "Output naming". |
| `SWEEP_ID` | fresh UTC ISO-8601 timestamp (`+ -LABEL` if set) | Override only to resume/share a dir. Should match `<YYYY-MM-DDTHH-MM-SSZ>[-<kebab-slug>]`; `sweep.sh` warns on deviations. |

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
| `GET /users/{userId}/info` | — | `getInfo({ ensureSynced: false })` (local read, no sync) |
| `POST /users/{userId}/send` | `{"destination":"<spark addr>","amountSats":N}` | `prepareSendPayment` + `sendPayment` |
| `POST /users/{userId}/receive` | `{}` | `receivePayment(SparkAddress)` (address generation only) |

Each request does a fresh `connect → op → disconnect` using
`defaultServerConfig` (server mode). All shared resources — MySQL pool,
HTTP client (SSP, LNURL, JWT, chain), operator gRPC, Breez-backend gRPC —
are bundled into one `SdkContext` threaded into every build. All
requests run fully in parallel — there is no per-user serialization
(safe: operator-enforced single-spend + coordinator reconciliation, see
`DESIGN.md`). No request handler syncs; syncs are explicit and confined to
the `fund` / `seed-senders` / `trace-sync` steps (see `DESIGN.md`).
