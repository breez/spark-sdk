# Design notes — server-side SDK bench

How the harness is structured and why. The README covers how to run
it; this covers what the numbers mean.

## Goal

Estimate **per-pod RPS capacity** for the SDK in a server-side,
multi-tenant deployment. Three numbers, pinned to a documented host:

1. Per-op latency p50/p95/p99 at sustained RPS.
2. RAM footprint at sustained RPS.
3. Max RPS before client-side p99(send) doubles vs. the lowest stable
   point in the sweep — derived "headroom" number.

Latency is reported **per op**, not blended. The op mix on the load
generator is a stress knob to drive load, not a measurement choice;
any consumer can compose their own mix from the per-op numbers.

## Shape: HTTP server + open-loop loadgen

A Ktor HTTP server with three endpoints (`info`, `send`, `receive`)
plus a separate open-loop load generator. We picked this over a
"spin up many in-process SDK instances" benchmark because:

1. It's request-driven — what predicts user experience, even with an
   in-process pool layer underneath.
2. It doubles as a working server-side integration template.
3. "Always-on vs session-scoped" stops being two scenarios and
   becomes one tunable knob (pool strategy). The current bench pins
   it to "no pool".

Wire-protocol choice (HTTP vs gRPC vs GraphQL) is incidental: SDK +
MySQL + operator round-trips dominate, and protocol overhead is small
and similar across all three. HTTP/Ktor is the path of least
friction in Kotlin/JVM.

## Per-request SDK lifecycle (no pooling — worst-case baseline)

Each request runs the same flow:

```
1. Acquire per-userId lock
2. Derive seed: HMAC-SHA512(MASTER_SECRET, userId) → 64 bytes
3. SdkBuilder(config, Seed.Entropy(…)).withMysqlBackend(SHARED_CONFIG).build()
4. sdk.<op>(…)
5. sdk.disconnect()
6. Release lock; respond
```

A single shared `MysqlStorageConfig` is reused across requests;
multi-tenancy in the SDK scopes each instance to its own identity
inside the DB.

This is deliberately the **worst case**: every request pays cold-
start (SDK build + initial sync + operator TLS handshakes). Pinning
this baseline gives a defensible **lower bound for capacity, upper
bound for latency**. SDK-level optimizations in flight upstream
(shared MySQL pool, shared operator connections) push these numbers
favorably without any harness change; an in-process SDK pool would
push them further.

## Per-userId lock (multi-tenancy safety)

Two concurrent requests for the same `userId` would build two SDK
instances with the same seed, racing reads/writes against the same
multi-tenant MySQL identity rows. The bench serializes per user with
a `ConcurrentHashMap<String, Mutex>`; different user-ids run in
parallel, same user-id calls serialize.

Implication for skewed user distributions (zipf): hot user-ids
serialize, so *delivered* RPS at the server is lower than offered
RPS. The headline reads as capacity under realistic skew, not raw
parallel throughput.

## Endpoints — semantics worth pinning

| Endpoint | Op | Notes |
|---|---|---|
| `GET /users/{userId}/info` | `getInfo({ ensureSynced: true })` | Sync forced — see below. |
| `POST /users/{userId}/send` | `prepareSendPayment` + `sendPayment` | Reported as a single number. |
| `POST /users/{userId}/receive` | `receivePayment(SparkAddress)` | **Address generation only.** |

- `/info` always uses `ensureSynced=true`. Without it, a fresh per-
  request SDK returns a meaningless balance. `ensureSynced=true`
  forces a sync only on the *first* call after SDK init — but in the
  no-pool baseline every request *is* a first call, so every `/info`
  pays the full sync cost.
- `/send` latency includes both `prepareSendPayment` and
  `sendPayment`. Single number matches what a real handler does.
- `/receive` is **address generation only** — `receivePayment` returns
  a Spark address; nothing actually arrives during the measurement
  window. The number is the cost of producing a deposit destination,
  not end-to-end receive cost. `RESULTS.md` should flag this.

**Payment method:** both `/send` and `/receive` use Spark transfers
(Spark address as destination). Closed-loop on regtest, deterministic,
no Lightning routing dependencies (regtest's Lightning network is
limited), and matches the SDK's most efficient payment path.

## Open-loop load generator

Dispatches at the **target RPS regardless of completion**: computes
`intervalNs = 1e9 / rps` and sleeps until the next dispatch tick. If
the server can't keep up, the in-flight queue grows; this surfaces
backpressure cleanly instead of being hidden. Closed-loop ("wait for
response before firing the next") would silently cap RPS at
1/latency and hide cliffs.

A hard cap (`--max-in-flight`, default 5000) prevents unbounded queue
growth: dispatches over the cap are recorded as `dropped=true` rows
in `latency.jsonl` and counted but not sent.

Sampling:
- `info` and `receive` user-ids come from a pool of size `--users`
  (default 10000), uniformly or zipf.
- `send` source user-ids are picked **round-robin** from the K
  reserved sender wallets (keeps closed-loop funding balanced).
  Destination is always the treasurer's Spark address, fetched once
  at startup and cached.

## Warmup + low-RPS exclusion

JVM-specific gotchas the loadgen handles:

- **Warmup.** Default `--warmup-secs=60`. Samples in that window are
  recorded with `warmup=true` in `latency.jsonl` but excluded from
  headline percentiles.
- **Low-RPS exclusion.** The HotSpot JIT optimizes hot paths only
  after roughly 10k invocations. At 1 RPS × 60s = 60 invocations,
  p99 reflects warmup, not steady-state. The default sweep starts at
  50 RPS for that reason.

## Closed-loop funding

`/send` always targets the treasurer; senders drain at `payment_sats`
per send, treasurer fills at the same rate. **System total** stays
constant (spark transfer fees are zero); per-wallet balances drift, so a
pre-sweep step seeds senders from the treasurer to a workload-sized
starting balance.

```
faucet → treasurer ←──── (loadgen `/send` destination)
            │
            ▼ (one-shot `seed-senders`)
   senders pool (K wallets) ──→ loadgen picks round-robin for `/send` source
```

Reserved user-ids derived from the same HMAC scheme as everything
else: `__treasurer__` and `__sender_0__` … `__sender_{K-1}__`.

`MASTER_SECRET` defaults to `breez-bench` and is reused across runs
on purpose: wallets persist between sessions, so the faucet is hit
only when the treasurer drops below threshold (in practice, once
after a long idle period). Use a different secret only when
intentionally starting from a clean wallet set.

### Workload-sized funding

`scripts/compute_funding.py` derives per-sender top-up and treasurer
target from the sweep config:

```
sends_per_sender   = sum(rps × duration_s) × send_mix_frac / K
per_sender_sats    = ceil(sends_per_sender × payment_sats × SAFETY)        # default 2.0
treasurer_target   = max(K × per_sender_sats × BUFFER, FLOOR)              # defaults 1.5, 50000
```

The Lightspark regtest faucet enforces a 50_000-sat per-call cap and
a 1_000-sat floor; `fund` mode chunks larger top-ups and waits for
each on-chain deposit to be claimed before requesting the next.

## Output layout

Everything is written to `out/<sweep-id>/` (gitignored):

```
out/<sweep-id>/
  manifest.json            sweep config + host info + funding budget
  RESULTS.md               headline tables (read this)
  summary.json             full structured per-step breakdown
  fund.log seed.log        pre-sweep step output (skipped if no-op)
  rps-50/
    server.log             server stdout/stderr
    loadgen.log            loadgen stdout/stderr
    requests.jsonl         server-side per-request timings (op, durMs, error)
    metrics.jsonl          1Hz process samples
    latency.jsonl          client-side per-request timings + warmup + dropped flags
  rps-100/  ...
```

The committed artifact is the harness itself; outputs are point-in-
time and live wherever the numbers are shared.

## Two latency views

- **`requests.jsonl` (server-side):** handler duration only, no
  network leg — the SDK + MySQL + operator cost.
- **`latency.jsonl` (client-side):** end-to-end including network
  round trip. Comparing the two surfaces network/TLS overhead.

Server-side is the better source for **error categorization** because
it carries the SDK's actual exception types; client-side mostly sees
`http_5xx` wrappers plus transport-layer issues.

## Metrics

A 1Hz background sampler emits `metrics.jsonl`. `-1` in any numeric
field means "unavailable on this platform / this tick" rather than
zero — aggregators filter `-1` rather than treating it as a real
value.

| Field | Linux | macOS |
|---|---|---|
| `rss_kb` | `/proc/self/status` | `ps -o rss=` |
| `heap_used_bytes` / `heap_total_bytes` | `Runtime` | same |
| `thread_count` | `ThreadMXBean` | same |
| `fd_count` | `UnixOperatingSystemMXBean` | same |
| `mysql_conns` | `INFORMATION_SCHEMA.PROCESSLIST` filtered by DB | same |
| `remote_tcp_sockets` | `/proc/self/net/tcp{,6}` | unavailable |
| `process_cpu_load` / `host_cpu_load` | `OperatingSystemMXBean` | same |
| `available_processors` | `Runtime` | same |

`mysql_conns` is server-authoritative (`COUNT(*)` from `PROCESSLIST`
filtered by the bench DB); multiple bench processes sharing the DB
would over-count. Fine for the single-process bench.

`remote_tcp_sockets` includes ephemeral TIME_WAIT — those still
consume local ports, which is the failure mode (port exhaustion at
high RPS during cold-start churn). Linux only because there's no JVM
API for it on macOS, and `lsof` is too slow once the process
accumulates a few hundred FDs (same reason FDs use the JVM bean).
**Real measurements should be taken on Linux**; macOS is fine for
harness development.

## Aggregator

`scripts/aggregate.py` walks `out/<sweep-id>/rps-N/`, drops warmup
samples (using the loadgen's `warmup` flag as ground truth), and
computes:

- Per-op p50/p95/p99 (linear-interpolated), client and server views,
  successful requests only.
- Process metrics summary stats over the post-warmup window.
- **Headline `max_safe_rps`** = highest swept RPS where client-side
  p99(send) is < 2× the lowest stable p99(send). "Stable" requires
  ≥30 samples. Threshold-doubling, not an absolute SLA, so the bench
  characterizes the host without hard-coding a target.
- Errors bucketed into named categories (`mysql_pool_exhausted`,
  `port_exhaustion`, `connect_timeout`, `operator_unreachable`,
  `operator_other`, `storage_other`, `request_timeout`,
  `connect_refused`, `http_5xx`, `http_4xx`, `other`). Pattern order
  matters: root-cause patterns are checked before wrapper exception
  types, so "Too many connections" inside a `StorageException` is
  attributed to `mysql_pool_exhausted`, not `storage_other`.

Stdlib only — no numpy / pandas / matplotlib.

## RPS sweep driver

`scripts/sweep.sh` restarts the server fresh per step. **Per-step
restart matters**: cold-start latency is part of this baseline;
reusing one server across steps would amortize that and overstate
capacity. An inter-step sleep (default 5s) lets TIME_WAIT sockets
drain before the next step opens its own.

The sweep writes `manifest.json` upfront so an aborted sweep still
leaves provenance behind, then runs the (idempotent) `fund` and
`seed-senders` steps before any RPS step. Server start is gated on a
healthcheck poll. SIGTERM is preferred over SIGKILL on shutdown so
JSONL writers flush via the process's shutdown hook; SIGKILL is the
fallback after a 15s grace.

## Implementation notes

- **Seed derivation.** HMAC-SHA512(`MASTER_SECRET`, userId) → 64
  bytes → `Seed.Entropy(…)`. Raw entropy avoids carrying a BIP39
  wordlist; the SDK accepts both.
- **Idempotency keys.** `SendPaymentRequest.idempotencyKey` is left
  null (one fresh key per SDK call), so the SDK doesn't dedupe sends
  to the same destination across the load run.
- **JSONL writer.** A daemon thread per file drains an unbounded
  `LinkedBlockingQueue`; per-write flush keeps the file readable
  mid-run. Bench volume (≤ a few thousand records/sec, small records)
  makes the unbounded queue free.
- **Run-id format.** Filesystem-safe ISO-8601 (`2026-05-08T15-23-45`
  — `-` instead of `:`). `:` is path-hostile on Windows and confusing
  on macOS.
