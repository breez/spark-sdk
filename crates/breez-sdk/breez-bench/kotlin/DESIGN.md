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

## Per-request SDK lifecycle (no SDK pool — worst-case baseline)

Each request runs the same flow:

```
1. Derive seed: HMAC-SHA512(MASTER_SECRET, userId) → 64 bytes
2. SdkBuilder(defaultServerConfig(REGTEST), Seed.Entropy(…))
     .withSharedContext(SHARED_CONTEXT)   // DB pool + SSP/LNURL/JWT/chain HTTP + operator gRPC + Breez gRPC
     .build()                             // chain service unset → built over the context HTTP client
3. sdk.<op>(…)
4. sdk.disconnect()
5. Respond
```

All requests run fully in parallel, including concurrent ones for the
same `userId` (no per-user lock — see [No per-userId lock](#no-per-userid-lock-it-was-an-artifact-removed)).

The bench builds with **`defaultServerConfig`** (server mode). That sets
`backgroundTasksEnabled = false`: no per-request periodic sync loop, Spark
background processor, RT-sync websocket, lightning-address recovery, or
private-mode init read — all wasted on a build → one op → disconnect
lifecycle. The trade: `ensureSynced=true` is rejected; the host syncs
explicitly (see [Where syncs happen](#where-syncs-happen)).

One process-wide **`SdkContext`** (`newSharedSdkContext`) is built once at
startup and threaded into every `SdkBuilder`. It bundles the shared HTTP
client (SSP, LNURL, JWT, chain service), the operator gRPC channels, the
Breez-backend gRPC client, and the MySQL pool.

Without this sharing, each request would open its own MySQL pool, redo
TCP+TLS+HTTP/2 to the SSP, and redial every operator — that dominates
latency and exhausts FDs / ephemeral ports under load. Multi-tenancy
still scopes each instance to its own identity inside the shared DB.

## No per-userId lock (it was an artifact, removed)

An earlier version serialized same-`userId` requests through a
`ConcurrentHashMap<String, Mutex>`, on the assumption that two
concurrent SDK instances with the same seed would race the shared
MySQL identity rows. **That serialization is gone — it was never a
correctness requirement and it dominated/distorted the measurement.**

Concurrent same-wallet instances are safe without it: operators
enforce single-spend via 2-of-3 FROST co-signing, the SDK's claim path
retries on the race, and the MySQL tree store writes idempotently and
reconciles against the coordinator on every fresh build.

With the lock, the bulk of `/send` latency was threads queued on the
per-sender mutex — an artifact of routing all sends through a small
reserved set, not real per-pod cost. Without it, concurrent same-sender
sends instead contend on that wallet's leaves at the operators: with
enough leaves they parallelize; with few they fail/retry on contention.
That is a *workload* property (how many distinct sender identities, how
many leaves each holds), now visible instead of hidden behind a lock.

## Endpoints — semantics worth pinning

| Endpoint | Op | Notes |
|---|---|---|
| `GET /users/{userId}/info` | `getInfo({ ensureSynced: false })` | Pure local read, no sync — see below. |
| `POST /users/{userId}/send` | `prepareSendPayment` + `sendPayment` | Reported as a single number. |
| `POST /users/{userId}/receive` | `receivePayment(SparkAddress)` | **Address generation only.** |

- `/info` is a **pure local balance read** — no sync. `getInfo` reads
  from the spark wallet (loaded from the tree store on build); a real
  server-mode deployment never syncs defensively on a read (incoming
  syncs are webhook-driven, see [Where syncs happen](#where-syncs-happen)).
  The headline `/info` number is per-request **build + local read** — the
  real steady-state server cost, not a forced cold sync.
- `/send` latency includes both `prepareSendPayment` and `sendPayment`,
  one number, matching a real handler. No pre-sync: the sender only sends
  (never receives) mid-run, and `transfer` self-refreshes leaves from
  operators on contention — so concurrent same-sender requests (now
  unserialized) reconcile against the operators rather than needing a
  pre-sync, at the cost of contention when a sender has few leaves.
- `/receive` is **address generation only** — `receivePayment` returns
  a Spark address; nothing actually arrives during the measurement
  window. The number is the cost of producing a deposit destination,
  not end-to-end receive cost. `RESULTS.md` should flag this.

## Where syncs happen

Server mode makes sync explicit, so it's worth pinning.

**During an RPS step: never.** `/info` is a local read, `/receive` is
address generation, `/send` is the payment round-trip itself. The only
sync a real server would do here is webhook-driven on an incoming
payment, and the bench's closed loop has no wallet that is both receiving
and balance-read mid-run (senders only send; the treasurer only
accumulates and is never read during the window) — so the absent webhook
plane costs no fidelity.

**Out of band: explicit `syncWallet()`.** The `fund`, `seed-senders`,
and `trace-sync` steps need real incoming/on-chain state, so they use the
`syncedInfo()` helper (`syncWallet()` + local `getInfo`). Same `Full`
forced sync `ensureSynced=true` used to trigger — including the
treasurer's known multi-minute backlog — so funding timing is unchanged.

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

## Lightning support

Optional, opt-in via the mix. The whole layer is disabled when the mix
contains no `*_ln` label — pure-spark sweeps are byte-identical to the
pre-Lightning bench.

### Op-mix vocabulary carries the method

`OpSampler` already accepts arbitrary weighted keys, so the percentage
control is just the mix weights — no separate `--ln-ratio` flag. Labels:

| Label | Endpoint | Behavior |
|---|---|---|
| `info` | `GET /info` | Pure local read, no sync. |
| `send` (alias `send_spark`) | `POST /send` | Spark transfer → treasurer Spark addr. Zero-fee, closed-loop. |
| `send_ln` | `POST /send` | Pay a pre-minted bolt11 invoice via real LN (`preferSpark=false`). SSP-routed; non-zero fee; not sat-conserving. |
| `receive` (alias `receive_spark`) | `POST /receive` | Local Spark-address generation, no SSP roundtrip. |
| `receive_ln` | `POST /receive` | Mint a real bolt11 invoice on the user's wallet (one SSP roundtrip + Shamir preimage-share with operators). |

Bare `send` / `receive` keep meaning the spark variants so the documented
default `info=40,receive=30,send=30` and every legacy sweep config are
unchanged. The server infers spark-vs-LN per request from
`prepared.paymentMethod` — no wire-shape change to `SendBody`.

### Why bolt11 doesn't slot into the spark loop

`send_spark` works because the treasurer's Spark address is a static,
reusable destination — fetched once, every send reuses it. **bolt11
invoices are single-use**: every LN send needs a fresh invoice the
receiver actively minted. That pulls a minting wallet into the hot path
of every send.

### Pre-minted invoice pool

Solution: pre-mint the entire run's worth of invoices in a setup step
(`mint-invoices` mode), one file, one invoice per line. The loadgen
consumes one invoice per `send_ln` dispatch; **pool exhaustion is a
hard-stop** (the sweep driver sizes the pool from `total_send_ln ×
SEED_SAFETY`, so running out means the bench was undersized and the
data is noise — fail loudly with a non-zero exit, don't silently
degrade).

Minting throughput is the constraint. One SDK serially can't keep up
with sweep-scale pools (each mint = one SSP roundtrip + Shamir
preimage-share storage with operators, ~hundreds of ms each;
hundreds of thousands at top-of-sweep). Two parallelism layers:

1. **Bank pool.** `BANKS` reserved `__bank_n__` wallets — distinct
   identities so concurrent `receivePayment` calls don't serialize on
   a single MySQL identity.
2. **Bounded concurrency.** `MINT_PARALLELISM` in-flight mints (default
   20). The SSP is the actual bottleneck; banks just keep MySQL from
   being the bottleneck.

Bank wallets need **no funding** (receiving requires no balance).
Unpaid bolt11 invoices are **sync-inert** — they don't trigger the O(N)
slow-sync pathology that paid/received transfers do. So holding a
large pre-minted pool is cheap regardless of bank count. Payments that
do land on a bank during the run accumulate as unclaimed transfers,
never claimed during the measurement, written off at end of run.

### Real LN routing, immediate return, non-zero fees

`send_ln` uses `SendPaymentOptions.Bolt11Invoice(preferSpark=false,
completionTimeoutSecs=0u)`:

- **`preferSpark=false`** — real Lightning routing via the SSP, not the
  Spark fast-path. This is what the partner deploys in production; the
  Spark fast-path is an optimization, not the measurement target.
- **`completionTimeoutSecs=0u`** — `sendPayment` returns immediately
  with `Pending`. Same dispatch-latency semantics as the spark `/send`
  (which also returns immediately for spark-address transfers), so the
  numbers are directly comparable. Actual LN settlement happens async
  and is not measured here.

Expect `send_ln` to be meaningfully slower than `send_spark` (extra
SSP roundtrips on both prepare and send); surfaced as its own table
row, not blended.

### Fees, funding, and the broken closed loop

Real LN fees are non-zero (SSP-determined; ~few sats on regtest in
practice). The closed-loop "system sats constant" invariant **does not
hold** for LN — senders drain by `payment_sats + ln_fee` per send;
receivers accumulate the full `payment_sats`, which they never claim 
within the measurement. The bench:

1. **Probes the SSP fee once at setup.** `mint-invoices` mints one
   extra invoice and runs `prepareSendPayment` between two bank
   wallets — never calls `sendPayment`, so the probe invoice stays
   unpaid. The quoted `lightningFeeSats` is written to `<out>.fee`
   and printed as `[mint] ln_fee_sats=<N>`.
2. **Sizes sender prefunding from the probed fee.**
   `compute_funding.py --ln-fee-sats=N` adds
   `total_send_ln × (payment_sats + N) × SAFETY` to the per-sender
   drain. The sweep driver scales the probed fee by `LN_FEE_SAFETY`
   (default 1.5) before passing it in — headroom for SSP fee drift
   mid-run.
3. **Doesn't try to recycle.** Funds stranded on bank/treasurer
   wallets at end of run aren't reclaimed within a sweep. The faucet
   replenishes net leakage between runs on the next `make fund`.
   Post-run recovery tooling is out of v1 scope.

### Synthetic send-class rollup for the headline

The headline `max_safe_rps` is defined on client-side `p99(send)`.
With split labels the literal `send` bucket may be absent. The
aggregator emits a synthetic `_send_rollup` = union of all `send*`
durations (and `_receive_rollup` likewise) and `derive_p99_doubling`
prefers the rollup, falling back to literal `send` for legacy data. The
per-op table iterates only literal labels — distinct payment paths
render as distinct rows (a `send_ln` blended into a single `send`
percentile would be bimodal junk).

## Output layout

Everything for one sweep is written to `out/<sweep-id>/` (gitignored).

**Sweep-id rule** (enforced loosely by `sweep.sh`):

```
<sweep-id> = <UTC-ISO-8601-timestamp>[-<kebab-slug>]
e.g.  2026-05-20T11-45-00Z
      2026-05-20T11-45-00Z-mysql-recycle      (LABEL=mysql-recycle make run)
```

```
out/<sweep-id>/
  manifest.json            sweep config + host info + funding budget + MASTER_SECRET (out/ is gitignored)
  driver.log               live [sweep]/[fund]/[seed]/[loadgen] output (tee'd by `make run`)
  RESULTS.md               headline tables (read this)
  summary.json             full structured per-step breakdown
  fund.log seed.log        pre-sweep step output (skipped if no-op)
  rps-50/
    server.log             server stdout/stderr
    loadgen.log            loadgen stdout/stderr
    requests.jsonl         server-side per-request timings (op, durMs, error)
    metrics.jsonl          1Hz process samples
    latency.jsonl          client-side per-request timings + dropped flag
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

`scripts/aggregate.py` walks `out/<sweep-id>/rps-N/` and computes:

- Per-op p50/p95/p99 (linear-interpolated), client and server views,
  successful requests only.
- Process metrics summary stats over the full step window.
- **Headline `max_safe_rps`** = highest swept RPS where client-side
  p99(send) is < 2× the lowest stable p99(send). "Stable" requires
  ≥30 samples. Threshold-doubling, not an absolute SLA, so the bench
  characterizes the host without hard-coding a target.
- Errors bucketed by category (mysql / operator / transport / http);
  root-cause patterns are checked before wrapper exception types so a
  "Too many connections" buried inside a `StorageException` resolves
  to `mysql_pool_exhausted`, not `storage_other`.

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
