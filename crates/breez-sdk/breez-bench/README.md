# Breez SDK Performance Benchmarks

This crate provides performance benchmarks for the Breez SDK, specifically measuring payment/transfer times and the impact of leaf denomination swaps.

## Running Benchmarks

### Regtest (Default)

Uses temporary wallets and automatic funding via faucet:

```bash
cargo run -p breez-sdk-bench
```

### Mainnet

Uses persistent wallets with mnemonics stored in phrase files:

```bash
# First run will create phrase files with new mnemonics
cargo run -p breez-sdk-bench -- \
  --network mainnet \
  --sender-data-dir ~/.breez-bench/sender \
  --receiver-data-dir ~/.breez-bench/receiver

# Set API key for mainnet
export BREEZ_API_KEY="your-api-key"
```

**First run on mainnet:**
- If no funds exist, the benchmark will print the sender address and exit
- Fund the sender wallet, then run again
- All funds are consolidated to sender before benchmark starts
- All funds are returned to sender after benchmark completes

### Custom Parameters

```bash
# Custom seed and payment count
cargo run -p breez-sdk-bench -- --seed 42 --payments 200

# Adjust amount range
cargo run -p breez-sdk-bench -- --min-amount 100 --max-amount 2000

# Adjust delay between payments
cargo run -p breez-sdk-bench -- --min-delay-ms 500 --max-delay-ms 5000

# Disable return payments (receiver sending back to sender)
cargo run -p breez-sdk-bench -- --return-interval 0

# Run deterministic edge-case scenarios
cargo run -p breez-sdk-bench -- --scenario edge-cases
```

## Configuration

### CLI Arguments

| Argument | Description | Default |
|----------|-------------|---------|
| `--network` | Network to use (regtest, mainnet) | `regtest` |
| `--sender-data-dir` | Sender wallet data directory (mainnet only) | Required for mainnet |
| `--receiver-data-dir` | Receiver wallet data directory (mainnet only) | Required for mainnet |
| `--seed` | Random seed for reproducible benchmarks | `12345` |
| `--payments` | Number of payments to execute | `100` |
| `--min-amount` | Minimum payment amount in sats | `100` |
| `--max-amount` | Maximum payment amount in sats | `2000` |
| `--min-delay-ms` | Minimum delay between payments | `500` |
| `--max-delay-ms` | Maximum delay between payments | `3000` |
| `--return-interval` | How often receiver sends funds back (0 to disable) | `5` |
| `--scenario` | Scenario preset | `random` |

### Environment Variables

| Variable | Description | Required |
|----------|-------------|---------|
| `BREEZ_API_KEY` | Breez API key | Mainnet only |
| `FAUCET_URL` | Regtest faucet URL | No (has default) |
| `FAUCET_USERNAME` | Faucet basic auth username | No |
| `FAUCET_PASSWORD` | Faucet basic auth password | No |
| `RUST_LOG` | Logging level | No (default: info) |

### Scenario Presets

- `random` - Seeded pseudo-random payment amounts and delays (default)
- `edge-cases` - Deterministic amounts that test various leaf configurations
- `small-payments` - Small amounts (less likely to need swaps)
- `large-payments` - Large amounts (more likely to need swaps)

## Output

The benchmark outputs statistical analysis of payment times:

```
Payment Performance Results (seed: 12345, n=100)
================================================
Total time:     Min: 45ms   Max: 2340ms   Mean: 312ms   StdDev: 456ms
  p50: 180ms   p75: 290ms   p90: 680ms   p95: 1200ms   p99: 2100ms

Breakdown:
  Without swap (n=72): p50: 120ms  p90: 210ms  p99: 380ms
  With swap (n=28):    p50: 890ms  p90: 1800ms p99: 2300ms
```

## Swap Detection

The benchmark distinguishes between:
- **Payment-time swaps**: Swaps triggered during payment when leaf denominations don't match (causes slowdowns)
- **Background optimization swaps**: Swaps triggered by background leaf optimization (not counted)

## Wallet Data Directory Structure

For mainnet, each wallet directory contains:
```
~/.breez-bench/sender/
├── phrase          # 12-word mnemonic (auto-generated if missing)
└── ...             # SDK storage files
```

**Security Note:** The phrase file contains your wallet mnemonic. Keep it secure and backed up.
