# Memory Leak Detection for Breez SDK Go Bindings

## Overview

This document describes the memory leak detection infrastructure for the Breez SDK Go bindings. The test harness helps identify memory leaks that occur when SDK instances are not properly cleaned up, particularly when `Disconnect()` is not called before garbage collection.

---

## Architecture

### Components

```
┌─────────────────────────────────────────────────────────────────┐
│                      Go Memory Test Harness                      │
├─────────────────────────────────────────────────────────────────┤
│  main.go          │ Orchestration, signal handling, reporting   │
│  config.go        │ CLI flags and configuration                 │
│  memory_tracker.go│ Periodic sampling, trend analysis, CSV      │
│  sdk_wrapper.go   │ SDK lifecycle management (connect/disconnect)│
│  payment_loop.go  │ Alice↔Bob payment exchange                  │
│  event_listener.go│ EventListener impl, listener churn          │
│  faucet.go        │ GraphQL faucet client for funding           │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Rust SDK (via UniFFI/CGO)                     │
├─────────────────────────────────────────────────────────────────┤
│  libbreez_sdk_spark_bindings.dylib/.so                          │
│  - BreezSdk instance management                                  │
│  - Background tokio tasks                                        │
│  - Event listener registry                                       │
└─────────────────────────────────────────────────────────────────┘
```

### Rust Baseline Test

A parallel Rust test (`breez-itest/tests/memory_test.rs`) provides a baseline for comparison, isolating Go-binding-specific leaks from Rust-side issues.

---

## Test Scenarios

### 1. Basic Payment Loop (Default)

Two SDK instances (Alice and Bob) exchange payments continuously. This establishes the baseline memory behavior under normal operation.

```bash
make run-go-memtest MEMTEST_ARGS="-duration 30m"
```

**What it tests:**
- Normal SDK operation memory footprint
- Payment processing overhead
- Event listener memory usage

### 2. Connect/Disconnect Cycles

Periodically disconnects and reconnects both SDK instances to verify that background tasks are properly cleaned up.

```bash
make run-go-memtest MEMTEST_ARGS="-reconnect-cycles -reconnect-every 50"
```

**What it tests:**
- Task cleanup on `Disconnect()`
- Storage handle release
- Channel/listener cleanup
- Arc reference counting

### 3. Event Listener Churn

Continuously adds and removes event listeners to test the internal BTreeMap storage cleanup.

```bash
make run-go-memtest MEMTEST_ARGS="-listener-churn"
```

**What it tests:**
- Listener registry cleanup
- Callback handle management
- FFI callback release

### 4. Combined Stress Test

```bash
make run-go-memtest MEMTEST_ARGS="-duration 1h -reconnect-cycles -reconnect-every 100 -listener-churn"
```

---

## Usage

### Prerequisites

1. Build the Go bindings:
   ```bash
   cd crates/breez-sdk/bindings
   make bindings-golang
   ```

2. Set faucet credentials (for regtest funding):
   ```bash
   export FAUCET_USERNAME="your_username"
   export FAUCET_PASSWORD="your_password"
   ```

### Building

```bash
cd crates/breez-sdk/bindings
make go-memtest
```

### Running

```bash
# Default: 1 hour test with payments every 5s
make run-go-memtest

# Custom duration and interval
make run-go-memtest MEMTEST_ARGS="-duration 30m -interval 10s"

# With reconnect cycles
make run-go-memtest MEMTEST_ARGS="-reconnect-cycles -reconnect-every 50"

# Direct execution
cd golang-memtest
DYLD_LIBRARY_PATH=../../../target/release ./memtest -duration 30m
```

### CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `-duration` | `1h` | Total test duration |
| `-interval` | `5s` | Time between payments |
| `-mem-interval` | `30s` | Memory sampling interval |
| `-amount` | `1000` | Satoshis per payment |
| `-reconnect-cycles` | `false` | Enable disconnect/reconnect testing |
| `-reconnect-every` | `100` | Payments between reconnect cycles |
| `-listener-churn` | `false` | Enable listener add/remove churn |
| `-pprof` | `false` | Enable pprof HTTP endpoint |
| `-pprof-port` | `6060` | Port for pprof server |
| `-heap-dump` | `false` | Write heap profile on exit |
| `-csv` | `""` | Export time-series to CSV file |

### Environment Variables

| Variable | Description |
|----------|-------------|
| `FAUCET_USERNAME` | Faucet basic auth username |
| `FAUCET_PASSWORD` | Faucet basic auth password |
| `FAUCET_URL` | Override faucet GraphQL endpoint |

---

## Output Format

### Periodic Output

During execution, the test prints memory samples:

```
[00:00:00] HeapAlloc=45.00MB Goroutines=42 Payments=0 Listeners=2
[00:00:30] HeapAlloc=47.12MB (+2.12MB) Goroutines=44 Payments=6 Listeners=2
[00:01:00] HeapAlloc=48.50MB (+1.38MB) Goroutines=44 Payments=12 Listeners=2
...
```

| Column | Description |
|--------|-------------|
| `HeapAlloc` | Go heap bytes allocated and in use |
| `(+X.XXmb)` | Delta from previous sample |
| `Goroutines` | Number of active goroutines |
| `Payments` | Cumulative successful payments |
| `Listeners` | Active event listeners |

### Final Report

On completion, a trend analysis report is generated:

```
=== Memory Trend Report ===
Time(min)  HeapAlloc    Delta      Rate(KB/min)  Goroutines  Payments
---------------------------------------------------------------
0.0        42.00MB      -          -             42          0
5.0        48.50MB      +6.5MB     +1300         44          60
10.0       55.00MB      +6.5MB     +1300         46          120
15.0       61.50MB      +6.5MB     +1300         48          180
...

--- Summary ---
Linear regression: 1300.0 KB/min (R²=0.94)
Heap: 42.00MB -> 85.00MB (max: 85.00MB)
Goroutines: 42 -> 52 (max: 52)
Total payments: 720

!!! LEAK DETECTED: Consistent linear growth: +1300.0 KB/min (R²=0.94)
```

---

## Interpreting Results

### Healthy Behavior

A healthy SDK shows:

| Metric | Expected Behavior |
|--------|-------------------|
| HeapAlloc | Stabilizes after warmup (first 2-5 minutes) |
| Goroutines | Constant count (±2 for GC fluctuations) |
| Linear regression slope | Near 0 KB/min |
| R² value | Low (< 0.5) indicating no linear trend |

**Example healthy output:**
```
Linear regression: 12.3 KB/min (R²=0.23)
Verdict: No significant memory leak detected
```

### Leak Indicators

**Memory Leak Detected When:**

1. **Linear memory growth**
   - Slope > 100 KB/min
   - R² > 0.7 (strong linear correlation)

   ```
   !!! LEAK DETECTED: Consistent linear growth: +1300.0 KB/min (R²=0.94)
   ```

2. **Goroutine accumulation**
   - Count doubles or keeps growing
   - Indicates spawned tasks not being cleaned up

   ```
   !!! LEAK DETECTED: Goroutine count doubled: 42 -> 98
   ```

3. **Post-reconnect growth**
   - Memory doesn't return to baseline after disconnect/reconnect
   - Each cycle adds permanent overhead

### Leak Severity Classification

| Slope (KB/min) | R² | Severity | Action |
|----------------|-----|----------|--------|
| < 50 | any | None | Normal operation |
| 50-100 | < 0.5 | Low | Monitor, may be GC timing |
| 50-100 | > 0.7 | Medium | Investigate with pprof |
| > 100 | > 0.7 | High | Leak confirmed, fix required |
| > 500 | > 0.8 | Critical | Severe leak, immediate fix |

---

## Advanced Profiling

### Using pprof

Enable the pprof HTTP endpoint for detailed heap analysis:

```bash
./memtest -pprof -duration 30m
```

In another terminal:

```bash
# Live heap profile
go tool pprof http://localhost:6060/debug/pprof/heap

# Goroutine profile (find leaked tasks)
go tool pprof http://localhost:6060/debug/pprof/goroutine

# CPU profile
go tool pprof http://localhost:6060/debug/pprof/profile?seconds=30
```

### Heap Dump Analysis

```bash
./memtest -heap-dump -duration 30m
# On exit: Heap profile written to: /tmp/memtest-xxx/heap-xxx.pprof

go tool pprof /tmp/memtest-xxx/heap-xxx.pprof
(pprof) top 20
(pprof) web  # Opens browser visualization
```

### CSV Export for Graphing

```bash
./memtest -csv=memory_report.csv -duration 1h
```

Import into Excel/Google Sheets for custom visualization:

| Column | Description |
|--------|-------------|
| `timestamp` | RFC3339 timestamp |
| `elapsed_sec` | Seconds since start |
| `heap_alloc_bytes` | HeapAlloc value |
| `heap_inuse_bytes` | HeapInuse value |
| `heap_objects` | Number of heap objects |
| `goroutines` | Goroutine count |
| `payments` | Payment count |
| `listeners` | Listener count |

---

## Rust Baseline Test

Run the equivalent Rust test for comparison:

```bash
# Default 10-minute test
cargo test -p breez-sdk-itest memory_baseline_test -- --ignored --nocapture

# Extended test with reconnect cycles
MEMTEST_DURATION_SECS=3600 \
MEMTEST_RECONNECT_CYCLES=1 \
MEMTEST_RECONNECT_EVERY=100 \
cargo test -p breez-sdk-itest memory_baseline_test -- --ignored --nocapture
```

### Comparing Go vs Rust Results

| Scenario | Go Leak | Rust Leak | Likely Cause |
|----------|---------|-----------|--------------|
| Yes | No | Go binding FFI cleanup |
| Yes | Yes | Rust SDK task management |
| No | Yes | Unlikely, investigate Rust |
| No | No | No leak present |

---

## Troubleshooting

### Test Won't Start

1. **Missing library**: Ensure `make bindings-golang` completed successfully
2. **Library path**: Set `DYLD_LIBRARY_PATH` (macOS) or `LD_LIBRARY_PATH` (Linux)
3. **Faucet credentials**: Check `FAUCET_USERNAME` and `FAUCET_PASSWORD`

### Payments Failing

1. **Insufficient funds**: Faucet may be rate-limited, wait and retry
2. **Network issues**: Check regtest environment connectivity
3. **Sync timeout**: Increase initial sync wait time

### High Variance in Results

1. **GC interference**: Run longer tests (1h+) to smooth out GC cycles
2. **System load**: Run on quiet machine, avoid concurrent heavy processes
3. **Warmup period**: Ignore first 5 minutes of data for trend analysis

---

## Files Reference

```
crates/breez-sdk/bindings/
├── Makefile                    # go-memtest, run-go-memtest targets
├── ffi/golang/                 # Generated bindings (gitignored)
│   ├── breez_sdk_spark/        # Main SDK bindings
│   └── breez_sdk_spark_bindings/
└── golang-memtest/             # Memory test harness (tracked in git)
    ├── config.go               # CLI configuration
    ├── event_listener.go       # EventListener implementation
    ├── faucet.go               # GraphQL faucet client
    ├── go.mod                  # Module with replace directive to ffi/golang
    ├── leak_detection.md       # This document
    ├── main.go                 # Entry point
    ├── memory_tracker.go       # Sampling and analysis
    ├── payment_loop.go         # Payment exchange logic
    ├── README.md               # Quick start guide
    └── sdk_wrapper.go          # SDK lifecycle helpers

crates/breez-sdk/breez-itest/
└── tests/
    └── memory_test.rs          # Rust baseline test
```
