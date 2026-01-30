# Memory Tests

Memory leak tests for comparing Go bindings vs native Rust SDK. Both track RSS and heap metrics.

**Prerequisites:**
- Faucet credentials: `FAUCET_USERNAME` and `FAUCET_PASSWORD`
- Wallet seeds (64 hex chars each): `ALICE_SEED` and `BOB_SEED`

Example seed generation:
```bash
export ALICE_SEED=$(openssl rand -hex 32)
export BOB_SEED=$(openssl rand -hex 32)
```

## Go Memory Test

Location: `crates/breez-sdk/bindings/golang-memtest/`

```bash
# Build
cd crates/breez-sdk/bindings && make go-memtest

# Run (from bindings dir)
make run-go-memtest MEMTEST_ARGS="-duration 30m -csv=output.csv"
```

### Go Flags

| Flag | Default | Description |
|------|---------|-------------|
| `-duration` | `1h` | Test duration |
| `-interval` | `5s` | Payment interval |
| `-mem-interval` | `30s` | Memory sampling interval |
| `-amount` | `1000` | Satoshis per payment |
| `-payment-type` | `spark` | `spark`, `lightning`, or `both` |
| `-reconnect-cycles` | `false` | Enable disconnect/reconnect cycles |
| `-reconnect-every` | `100` | Payments between reconnects |
| `-listener-churn` | `false` | Enable listener add/remove churn |
| `-frequent-sync` | `false` | Call sync_wallet on every cycle |
| `-payment-history` | `false` | Query payment history on every cycle |
| `-payment-history-limit` | `0` | Limit for payment history queries (0 = unlimited) |
| `-destroy-responses` | `false` | Call Destroy() on ListPayments responses |
| `-force-gc` | `false` | Force GC after each payment cycle |
| `-extra-instances` | `0` | Extra SDK instances (same seeds) |
| `-pprof` | `false` | Enable pprof HTTP endpoint |
| `-heap-dump` | `false` | Dump heap profile on exit |
| `-csv` | none | Export time-series to CSV file |

## Rust Memory Test

Location: `crates/breez-sdk/breez-itest/tests/memory_test.rs`

```bash
# Run
cargo test -p breez-sdk-itest memory_baseline_test -- --ignored --nocapture

# With options
MEMTEST_DURATION_SECS=1800 MEMTEST_CSV_FILE=output.csv cargo test -p breez-sdk-itest memory_baseline_test -- --ignored --nocapture
```

### Rust Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MEMTEST_DURATION_SECS` | `600` | Test duration in seconds |
| `MEMTEST_PAYMENT_TYPE` | `spark` | `spark`, `lightning`, or `both` |
| `MEMTEST_RECONNECT_CYCLES` | unset | Set to enable reconnect cycles |
| `MEMTEST_RECONNECT_EVERY` | `100` | Payments between reconnects |
| `MEMTEST_EXTRA_INSTANCES` | `0` | Extra SDK instances (same seeds) |
| `MEMTEST_FREQUENT_SYNC` | unset | Set to enable frequent sync calls |
| `MEMTEST_PAYMENT_HISTORY_QUERIES` | unset | Set to enable payment history queries |
| `MEMTEST_PAYMENT_HISTORY_LIMIT` | unset | Limit for payment history queries (unset = unlimited) |
| `MEMTEST_CSV_FILE` | none | Export time-series to CSV file |

## CSV Output Format

Both tests export comparable CSV with these key columns:
- `timestamp` - Wall clock time
- `elapsed_sec` - Seconds since start
- `rss_bytes` - Resident set size
- Heap metrics (Go: `heap_alloc_bytes`, Rust: `heap_allocated_bytes`)
- `payments` - Payment count

## Examples

```bash
# Quick 30-second smoke test (Go)
make run-go-memtest MEMTEST_ARGS="-duration 30s"

# Quick 30-second smoke test (Rust)
MEMTEST_DURATION_SECS=30 cargo test -p breez-sdk-itest memory_baseline_test -- --ignored --nocapture

# 1-hour test with reconnect cycles and CSV export (Go)
make run-go-memtest MEMTEST_ARGS="-duration 1h -reconnect-cycles -reconnect-every 100 -csv=go_results.csv"

# 1-hour test with extra instances (Rust)
MEMTEST_DURATION_SECS=3600 MEMTEST_EXTRA_INSTANCES=2 MEMTEST_CSV_FILE=rust_results.csv cargo test -p breez-sdk-itest memory_baseline_test -- --ignored --nocapture
```

## Generating Visualizations

Use the `scripts/generate_memtest_plots.py` script to create HTML visualizations from CSV results:

```bash
# Compare Go and Rust results
python scripts/generate_memtest_plots.py \
    --go go_results.csv \
    --rust rust_results.csv \
    --output plots.html \
    --title "1h Stress Test"

# Single test visualization
python scripts/generate_memtest_plots.py --go go_results.csv -o go_plots.html

# Full stress test example
python scripts/generate_memtest_plots.py \
    --go go_memtest_1h_stress.csv \
    --rust rust_memtest_1h_stress.csv \
    -o memtest_1h_stress_plots.html \
    -t "1h Stress Test (Lightning + Reconnects + Extra Instances)"
```

The generated HTML includes:
- Interactive charts for RSS and Heap memory over time
- Summary statistics (start/end values, growth)
- Dark theme styling
