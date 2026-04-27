# Concurrent Spark transfer benchmark — Node.js wasm

Counterpart to the Rust `concurrent-perf` bench (`crates/breez-sdk/breez-bench/src/bin/concurrent_perf.rs`). Measures send-payment throughput for `@breeztech/breez-sdk-spark/nodejs` against a real regtest network using the public Lightspark faucet.

## Setup

Requires Node.js with native module support for `better-sqlite3`. Tested on Node 22.

```sh
cd crates/breez-sdk/breez-bench/js
nvm use 22   # or any node >=18
npm install
```

The package depends on `../../../../packages/wasm` (the local wasm package). Rebuild the wasm package via `cargo xtask package wasm::node` if you change SDK source.

## Required env

- `FAUCET_USERNAME`, `FAUCET_PASSWORD` — Lightspark regtest faucet credentials.
- `BREEZ_API_KEY` — optional (regtest doesn't require one).

## Backends

The bench supports SQLite (default, single instance) and Postgres (single or multi-instance).

For Postgres, point `--sender-postgres` and `--receiver-postgres` at running databases. A local docker postgres works:

```sh
docker run -d --name spark-perf-pg -e POSTGRES_PASSWORD=postgres -p 5544:5432 postgres:16
docker exec spark-perf-pg psql -U postgres -c 'CREATE DATABASE bench_sender_node;'
docker exec spark-perf-pg psql -U postgres -c 'CREATE DATABASE bench_receiver_node;'
```

Multi-instance senders (`--sender-instances N` where N > 1) require `--sender-postgres` so all instances share one tree store.

## Run

SQLite, single instance:

```sh
FAUCET_USERNAME=… FAUCET_PASSWORD=… node concurrent_perf.js \
    --total-payments 50 --concurrency 6
```

Postgres, multi-instance shared wallet:

```sh
FAUCET_USERNAME=… FAUCET_PASSWORD=… node concurrent_perf.js \
    --total-payments 1000 --concurrency 6 --sender-instances 3 \
    --sender-postgres   "postgres://postgres:postgres@localhost:5544/bench_sender_node" \
    --receiver-postgres "postgres://postgres:postgres@localhost:5544/bench_receiver_node"
```

## Common flags

| flag | default | meaning |
|------|---------|---------|
| `--total-payments N` | 100 | how many spark transfers to run |
| `--concurrency N` | 6 | per-instance in-flight cap |
| `--sender-instances N` | 1 | number of SDK instances sharing the sender wallet (requires `--sender-postgres` if > 1) |
| `--min-amount`, `--max-amount` | 100, 2000 | per-payment sat range (random) |
| `--funding-buffer` | 1.5 | faucet over-fund factor |
| `--no-auto-optimize` | (off) | disable the leaf optimizer |
| `--multiplicity N` | (default) | optimizer multiplicity |
| `--keep-data` | (off) | keep sender/receiver data dirs after run |
| `--sender-data-dir`, `--receiver-data-dir` | (tmp) | reuse a wallet across runs |
| `--label TEXT` | none | echoed in the summary header |

## Output

Per-payment progress (`[OK]` / `[FAIL]`), then a summary block with throughput, latency percentiles, failure breakdown, and a 60-second-bucket histogram so degradation over time is visible.

The SDK log is written to `<sender-data-dir>/sdk.log` (and the receiver's equivalent). Use `--keep-data` to preserve them after the run for analysis.
