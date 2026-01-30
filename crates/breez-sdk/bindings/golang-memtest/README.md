# Memory Leak Test Harness for Go Bindings

This is a standalone Go program that exercises the Breez SDK continuously to detect memory leaks in the Go bindings.

## Background

Go's `Destroy()` on the SDK only frees the Rust Arc pointer - it does NOT call `Disconnect()`. If SDK instances are garbage collected without explicit `Disconnect()`, spawned tokio tasks keep running indefinitely:
- `periodic_sync()` (sdk.rs)
- `spawn_zap_receipt_publisher()` (sdk.rs)
- `FlashnetTokenConverter::spawn_refunder()` (flashnet.rs)
- `SparkWallet::BackgroundProcessor` tasks

Each leaked instance holds Arc refs, event listeners, and channel receivers.

## Building

From the `crates/breez-sdk/bindings` directory:

```bash
# Build Go bindings and memtest binary
make go-memtest
```

This will:
1. Build the Rust library (`libbreez_sdk_spark_bindings.dylib/.so`)
2. Generate Go bindings in `ffi/golang/`
3. Build the memtest binary

## Running

```bash
# Run with default settings (1 hour, payments every 5s)
make run-go-memtest

# Run with custom arguments
make run-go-memtest MEMTEST_ARGS="-duration 30m -interval 10s -reconnect-cycles"

# Or run directly
cd golang-memtest
DYLD_LIBRARY_PATH=../../../target/release ./memtest -duration 30m
```

## Test Scenarios

### 1. Basic Payment Loop (default)
Two SDK instances exchange payments continuously. This is the baseline for "normal" memory behavior.

### 2. Connect/Disconnect Cycles (`-reconnect-cycles`)
Every N payments, disconnect both SDKs and reconnect. This is the **primary leak detector** - verifies that spawned tasks are properly cleaned up after `Disconnect()`.

```bash
./memtest -reconnect-cycles -reconnect-every 50
```

### 3. Event Listener Churn (`-listener-churn`)
Continuously add and remove event listeners. Tests that the BTreeMap listener storage is properly cleaned up.

```bash
./memtest -listener-churn
```

## CLI Flags

| Flag | Default | Description |
|------|---------|-------------|
| `-duration` | `1h` | Test duration |
| `-interval` | `5s` | Payment interval |
| `-mem-interval` | `30s` | Memory sample interval |
| `-amount` | `1000` | Sats per payment |
| `-reconnect-cycles` | `false` | Enable disconnect/reconnect cycles |
| `-reconnect-every` | `100` | Payments between reconnects |
| `-listener-churn` | `false` | Enable listener add/remove churn |
| `-pprof` | `false` | Enable pprof HTTP endpoint |
| `-pprof-port` | `6060` | Port for pprof endpoint |
| `-heap-dump` | `false` | Dump heap profile on exit |
| `-csv` | `""` | Export time-series to CSV file |
| `-faucet-url` | `https://api.lightspark.com/graphql/spark/rc` | Faucet GraphQL URL |

## Environment Variables

- `FAUCET_USERNAME` - Faucet basic auth username
- `FAUCET_PASSWORD` - Faucet basic auth password

## Memory Measurement

The test tracks:
- **HeapAlloc** - Bytes allocated and still in use
- **HeapInuse** - Bytes in in-use spans
- **HeapObjects** - Number of allocated objects
- **NumGoroutine** - Number of goroutines
- **PaymentCount** - Total successful payments
- **ListenerCount** - Active event listeners

### Periodic Output
```
[00:00:00] HeapAlloc=45.00MB Goroutines=42 Payments=0 Listeners=2
[00:00:30] HeapAlloc=47.12MB (+2.12MB) Goroutines=44 Payments=6 Listeners=2
...
```

### Trend Report (on exit)
```
=== Memory Trend Report ===
Time(min)  HeapAlloc    Delta      Rate(KB/min)  Goroutines  Payments
---------------------------------------------------------------
0.0        42.00MB      -          -             42          0
5.0        48.50MB      +6.5MB     +1300         44          60
10.0       55.00MB      +6.5MB     +1300         46          120
...

--- Summary ---
Linear regression: 1300.0 KB/min (R²=0.94)
Heap: 42.00MB -> 68.00MB (max: 68.00MB)
Goroutines: 42 -> 52 (max: 52)
Total payments: 720

!!! LEAK DETECTED: Consistent linear growth: +1300.0 KB/min (R²=0.94)
```

## Profiling

Enable pprof for detailed heap analysis:

```bash
./memtest -pprof -duration 30m
```

Then in another terminal:
```bash
# Live heap profile
go tool pprof http://localhost:6060/debug/pprof/heap

# Goroutine profile
go tool pprof http://localhost:6060/debug/pprof/goroutine
```

Or dump heap on exit:
```bash
./memtest -heap-dump -duration 30m
# Analyze with: go tool pprof /tmp/memtest-xxx/heap-xxx.pprof
```

## Interpreting Results

### Healthy Behavior
- HeapAlloc stabilizes after warmup (first few minutes)
- Goroutine count remains stable
- Linear regression slope near 0 with low R²

### Leak Indicators
- **Linear memory growth** (positive slope > 100KB/min with R² > 0.7)
- **Goroutine accumulation** (count doubles or keeps growing)
- **Listener count mismatch** (more than expected after churn cycles)

## CSV Export

Export time-series data for external analysis:

```bash
./memtest -csv=memory_report.csv -duration 1h
```

The CSV contains columns:
- `timestamp` - RFC3339 timestamp
- `elapsed_sec` - Seconds since start
- `heap_alloc_bytes` - HeapAlloc
- `heap_inuse_bytes` - HeapInuse
- `heap_objects` - HeapObjects
- `heap_sys_bytes` - HeapSys
- `goroutines` - NumGoroutine
- `payments` - Payment count
- `listeners` - Listener count
