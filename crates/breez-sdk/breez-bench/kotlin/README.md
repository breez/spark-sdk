# Server-Side SDK Benchmarks (Kotlin/JVM)

An on-demand HTTP server that wraps the Breez SDK plus a load-generator
client, used to benchmark request-driven server-side deployments where
SDK instances are spun up per user request against a shared MySQL
backend (multi-tenant; identity-scoped per request).

Sibling of `../js/concurrent_perf.js` (the WASM/Node version).

## Status

- **Phase 1 (per-request smoke)**: implemented — single-shot run of the
  per-request flow, validates the SDK + MySQL + KMP path before HTTP is
  added in Phase 2.
- Phases 2–9: pending.

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

## Notes

- **No mnemonic file.** The seed is derived deterministically from
  `(master_secret, user_id)`; in a real deployment, the partner replaces
  this with their own secrets store lookup.
- **Multi-tenancy.** Many SDK instances safely share one MySQL database;
  each is scoped by its wallet identity public key (derived from the seed).
- **Per-request lifecycle.** Each smoke run does `connect → op → disconnect`.
  This is the v1 model the Phase 2 server will use for every HTTP request.
  Pooling (Phase 6) lets us trade memory for latency once the baseline is in.
