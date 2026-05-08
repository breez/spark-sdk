# Server-Side SDK Benchmarks (Kotlin/JVM)

An on-demand HTTP server that wraps the Breez SDK plus a load-generator
client, used to benchmark request-driven server-side deployments where
SDK instances are spun up per user request against a shared MySQL
backend (multi-tenant; identity-scoped per request).

Sibling of `../js/concurrent_perf.js` (the WASM/Node version).

## Status

- **Phase 1 (per-request smoke)**: implemented.
- **Phase 2 (HTTP server, 3 endpoints, per-request lifecycle)**: implemented.
- **Phase 3 (funding pipeline)**: implemented — `fund` (treasurer
  top-up via faucet) and `seed-senders` (one-shot top-up of K sender
  wallets from treasurer). Both validated end-to-end on regtest.
- **Phase 4 (load generator)**: implemented — open-loop HTTP load
  generator that dispatches at a target RPS regardless of completion,
  emits per-request JSONL, prints periodic progress.
- **Phase 5 (server-side metrics)**: implemented — per-request
  `requests.jsonl` (op, user, duration, error) plus a 1Hz background
  sampler emitting `metrics.jsonl` (RSS, JVM heap, threads, FDs,
  MySQL connection count, remote TCP socket count). Linux + macOS
  first-class via a platform shim.
- Phases 6–9: pending.

## Funding flow

The bench runs a closed loop: load generator's `/send` always targets
the treasurer's Spark address, so funds circulate
treasurer → sender → treasurer. The closed loop keeps the
**total system sats constant** (regtest fees are 0), but **per-wallet**
the senders still drain at the full payment amount per send. The
treasurer fills up by the same amount.

```
faucet  ───────►  treasurer  ◄─── (load gen `/send` destination)
                     │
                     ▼ (one-shot via `make seed-senders`)
              senders pool (K wallets)  ───── (load gen `/send` source)
```

**Sender pool runway** (how long a single seeding pass lasts before
senders are empty):

```
runway_seconds = (K × TARGET_SATS) / (RPS × payment_sats)
```

With defaults K=50, TARGET=50_000, payment=1 sat:

| RPS  | runway   |
|------|----------|
| 100  | ~7 hours |
| 500  | ~1.4 h   |
| 1000 | ~42 min  |

This is enough for the Phase 6 RPS sweep (~5 min per step). For
Phase 8 (24h leak run at sustained RPS), the prefund model isn't
enough — a continuous replenisher (treasurer→senders at the drain
rate) is needed and will be added before that phase.

Run outputs (per-request JSONL, metrics samples, summaries, and the
human-readable `RESULTS.md` digest) are written to `out/<run-id>/` and
**not committed** — point-in-time numbers live wherever they're shared,
not in git history. The committed artifact is the harness itself plus
this README; anyone who wants numbers re-runs the bench.

## Prerequisites

1. MySQL 8.0+ reachable from the host:
   ```bash
   docker run --rm -d --name bench-mysql -p 3306:3306 \
     -e MYSQL_ROOT_PASSWORD=password \
     mysql:8.0
   ```
   Create the shared database once:
   ```bash
   docker exec -i bench-mysql mysql -uroot -ppassword \
     -e "CREATE DATABASE IF NOT EXISTS breez_bench"
   ```
2. KMP bindings published to `mavenLocal`:
   ```bash
   make setup
   ```
   (Wait for `Local bindings published to mavenLocal`.)

## Smoke (Phase 1)

Exercises the per-request flow once:

1. Derive a 64-byte seed from `master_secret` + `user_id` via HMAC-SHA512.
2. Build the SDK with the shared MySQL backend.
3. `getInfo`.
4. Disconnect.

```bash
export MASTER_SECRET=any-string
MYSQL_URL='mysql://root:password@127.0.0.1:3306/breez_bench' make smoke-test
```

Or directly:

```bash
./gradlew run --console=plain --args="\
  --mode=smoke \
  --mysql-url=mysql://root:password@127.0.0.1:3306/breez_bench \
  --user-id=alice"
```

Expected output:

```
[smoke] user-id=alice  mysql=mysql://root:***@127.0.0.1:3306/breez_bench
[smoke] building SDK
[smoke] connect=NNNms
[smoke] balance=0 sats
[smoke] OK
```

## Server (Phase 2)

Ktor + Netty HTTP server with three endpoints:

| Endpoint | Body | Maps to |
|---|---|---|
| `GET /users/{userId}/info` | — | `getInfo({ ensureSynced: true })` |
| `POST /users/{userId}/send` | `{ "destination": "<spark addr>", "amountSats": <int> }` | `prepareSendPayment` + `sendPayment` |
| `POST /users/{userId}/receive` | `{}` | `receivePayment(SparkAddress)` (address generation only) |

Per-request flow: HMAC-derive seed → `SdkBuilder().withMysqlBackend().build()`
→ op → `disconnect()`. Same-`userId` requests serialize through a per-userId
mutex; different user-ids run in parallel.

```bash
export MASTER_SECRET=any-string
MYSQL_URL='mysql://root:password@127.0.0.1:3306/breez_bench' \
  PORT=8080 make run-server
```

Manual smoke (in another shell):

```bash
curl -s http://localhost:8080/users/alice/info
curl -s -X POST http://localhost:8080/users/alice/receive -H 'content-type: application/json' -d '{}'
curl -s -X POST http://localhost:8080/users/alice/send \
  -H 'content-type: application/json' \
  -d '{"destination":"<spark addr>","amountSats":1000}'
```

The server runs on regtest by default (Network.REGTEST). The `/info` and
`/receive` endpoints work without funding; `/send` will fail until the user
has been funded — Phase 3 adds the funding tooling (treasurer + sender pool
+ replenisher).

### Server-side metrics (Phase 5)

Every server run writes two JSONL streams to `out/<run-id>/`:

| File | Cadence | Fields |
|---|---|---|
| `requests.jsonl` | per-request | `ts`, `op`, `user_id`, `duration_ms`, `error` |
| `metrics.jsonl` | 1 Hz | `ts`, `rss_kb`, `heap_used_bytes`, `heap_total_bytes`, `thread_count`, `fd_count`, `mysql_conns`, `remote_tcp_sockets` |

`run-id` defaults to a fresh ISO-8601 timestamp; pass `RUN_ID=…` to
share a directory across server + loadgen for the same run.

```bash
# Server + loadgen sharing one run-id (out files end up in out/2026-05-08T15-00-00/):
RUN_ID=2026-05-08T15-00-00 \
  MYSQL_URL='mysql://root:password@127.0.0.1:3306/breez_bench' \
  MASTER_SECRET=any-string \
  make run-server &

RUN_ID=2026-05-08T15-00-00 TARGET_RPS=100 DURATION=2m \
  MYSQL_URL='mysql://root:password@127.0.0.1:3306/breez_bench' \
  MASTER_SECRET=any-string \
  make loadgen
```

Field notes:
- `mysql_conns` is `SELECT COUNT(*) FROM INFORMATION_SCHEMA.PROCESSLIST WHERE DB = <bench-db>`
  — server-authoritative count of connections open against the
  bench database. Coarse if multiple bench processes share a DB.
- `remote_tcp_sockets` counts non-loopback TCP sockets (any state,
  including TIME_WAIT) held by the server process. Surfaces ephemeral
  port exhaustion under cold-start churn — what we expect to bottleneck
  at high RPS in v1 before the SDK-level shared-pool optimizations land.
- `fd_count` is from `UnixOperatingSystemMXBean.getOpenFileDescriptorCount()`
  — works on both Linux and macOS, no subprocess.
- `-1` in any numeric field means "couldn't sample this tick" (transient
  failure, OS path missing); the sampler keeps running.

## Treasurer top-up (Phase 3a)

Walks the reserved treasurer wallet's (`__treasurer__`) balance up to
`TARGET_SATS` by repeatedly hitting the Lightspark regtest faucet and
waiting for each on-chain deposit to be claimed. Faucet caps each call
at 50_000 sats, so larger targets are split into chunks. Idempotent:
re-running with an already-funded treasurer exits without calling the
faucet.

```bash
export MASTER_SECRET=any-string
export FAUCET_USERNAME=...   # request from Lightspark
export FAUCET_PASSWORD=...
MYSQL_URL='mysql://root:password@127.0.0.1:3306/breez_bench' \
  TARGET_SATS=5000000 make fund
```

The treasurer is the on-ramp for the bench's funding pipeline; the
sender pool top-up below draws from it to keep the K active sender
wallets above their minimum threshold.

## Sender pool top-up (Phase 3b)

One-shot top-up: for each of K sender wallets (`__sender_0__` …
`__sender_{K-1}__`), if the wallet's balance is below `MIN_SATS`, the
treasurer Spark-transfers enough to bring it to `TARGET_SATS`. Run
once before a long bench run. Idempotent — re-running skips already-
funded senders. Bounded concurrency (default 5) so the treasurer SDK
isn't hammered with K simultaneous sendPayment calls.

```bash
SENDERS=50 MIN_SATS=10000 TARGET_SATS=50000 \
  MASTER_SECRET=any-string \
  MYSQL_URL='mysql://root:password@127.0.0.1:3306/breez_bench' \
  make seed-senders
```

Output: `[seed] OK  funded=N  skipped=M` summarises the work done.

## Load generator (Phase 4)

Open-loop HTTP load generator. Dispatches a new request every `1/RPS`
seconds regardless of whether earlier requests have completed —
deliberately chosen over closed-loop (fixed concurrency) so that
backpressure surfaces as in-flight queue growth and elevated latency,
which is the signal partners need to interpret the headline numbers.

- Picks senders round-robin from `__sender_0__` … `__sender_{K-1}__`
  for `/send`; the destination is the treasurer's Spark address
  (fetched once from the bench server at startup).
- Picks `/info` and `/receive` user-ids from the workload pool
  (`u0` … `u{N-1}`) by uniform or zipf sampling.
- Per-request JSONL written to `out/<run-id>/latency.jsonl` (per-line
  flush so the file is readable while the run is in flight).
- Hard cap on in-flight requests (default 5000); over-cap dispatches
  are recorded as `dropped:true` in the JSONL — surfaces "load gen
  itself can't keep up at this rate."
- Periodic progress logger every 5s while dispatching + during drain.

```bash
TARGET_RPS=100 DURATION=2m \
  USERS=10000 \
  MIX='info=40,receive=30,send=30' \
  DIST=uniform \
  WARMUP_SECS=60 \
  SENDERS=50 \
  PAYMENT_SATS=1 \
  make loadgen
```

End-of-run summary: `[loadgen] dispatched=N  dropped=M  actual_rps=R`.

## Notes

- **No mnemonic file.** The seed is derived deterministically from
  `(master_secret, user_id)`; in a real deployment, the partner replaces
  this with their own secrets store lookup.
- **Multi-tenancy.** Many SDK instances safely share one MySQL database;
  each is scoped by its wallet identity public key (derived from the seed).
- **Per-request lifecycle.** Each smoke run does `connect → op → disconnect`.
  This is the v1 model the Phase 2 server will use for every HTTP request.
  Pooling (Phase 6) lets us trade memory for latency once the baseline is in.
