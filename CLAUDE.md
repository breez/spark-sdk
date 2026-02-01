# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Dependencies

The following system dependencies are required:

```bash
# Debian/Ubuntu
apt-get install -y protobuf-compiler

# macOS
brew install protobuf

# Arch Linux
pacman -S protobuf
```

## Build Commands

```bash
make build              # Build workspace (excludes WASM)
make build-release      # Release build with LTO
make build-wasm         # Build for WASM target
```

## Testing

```bash
make cargo-test         # Run Rust unit tests
make wasm-test          # Run WASM tests (browser + Node.js)
make itest              # Integration tests (requires Docker)
make breez-itest        # Breez integration tests (requires faucet credentials)
```

Run a single test:
```bash
cargo test <test_name> -p <package>
```

## Code Quality

```bash
make check              # Run all checks (fmt, clippy, tests) - use before committing
make fmt-check          # Check formatting
make fmt-fix            # Fix formatting
make clippy-check       # Run clippy lints (cargo + WASM)
make clippy-fix         # Fix clippy issues
```

## Architecture

### Crate Structure

- **crates/breez-sdk/core** - Main SDK library with public API (`BreezSdk`)
- **crates/breez-sdk/common** - Shared utilities, LNURL support, networking, sync protocol
- **crates/breez-sdk/bindings** - UniFFI bindings for Go, Kotlin, Python, React Native, Swift
- **crates/breez-sdk/wasm** - WebAssembly bindings for JavaScript/TypeScript
- **crates/breez-sdk/cli** - Command-line interface for testing
- **crates/spark** - Low-level Spark protocol (addresses, signing, operators, tokens)
- **crates/spark-wallet** - High-level wallet operations wrapping Spark protocol
- **crates/xtask** - Custom build tasks (powers `make` commands via `cargo xtask`)

### Key Abstractions

- `Storage` trait - Pluggable persistence layer (default: SQLite)
- `Signer` trait - Cryptographic operations (FROST threshold signing)
- `BitcoinChainService` trait - Blockchain provider interface
- `EventEmitter` - Broadcasts `SdkEvent` (Synced, PaymentSucceeded, PaymentFailed, etc.)

### Data Flow

```
BreezSdk (API) → SparkWallet (wallet ops) → Spark (protocol) → Operators (gRPC)
     ↓
Storage → SyncedStorage → Breez Sync Service (multi-device)
```

## Updating SDK Interfaces

When changing the SDK's public interface, update these files:

1. **crates/breez-sdk/core/src/models.rs** - Add UniFFI macros to interface types
2. **crates/breez-sdk/wasm/src/models.rs** - Update exported structs/enums
3. **crates/breez-sdk/wasm/src/sdk.rs** - Update WASM interface
4. **packages/flutter/rust/src/models.rs** - Update mirrored structs/enums
5. **packages/flutter/rust/src/sdk.rs** - Update Flutter interface

## Workspace Configuration

- Rust edition 2024, MSRV 1.88
- Clippy: pedantic + suspicious + complexity + perf warnings enabled
- Release builds use LTO and `opt-level = "z"` for size optimization
- Uses `cargo xtask` for build automation (aliased in `.cargo/config.toml`)

---

## SDK Usage Guide (For Integrators)

This section is for developers integrating the Breez SDK into their apps.

### API Key (Required)

A Breez API key is required for the SDK to work. Request one for free at:
**https://breez.technology/request-api-key/**

### Installation

| Platform | Package |
|----------|---------|
| JavaScript/WASM | `npm install @breeztech/breez-sdk-spark` |
| React Native | `npm install @breeztech/breez-sdk-spark-react-native` |
| Python | `pip install breez-sdk-spark` |
| Go | `go get github.com/breez/breez-sdk-spark-go` |
| C# | `dotnet add package Breez.Sdk.Spark` |
| Swift | SPM: `https://github.com/breez/breez-sdk-spark-swift.git` |
| Kotlin | Maven: `https://mvn.breez.technology/releases` |
| Flutter | Git: `https://github.com/breez/breez-sdk-spark-flutter` |
| Rust | Git: `https://github.com/breez/spark-sdk` |

### Quick Start

See working examples in `docs/breez-sdk/snippets/` - these are compiled/tested and always up to date:

| Task | TypeScript | Rust |
|------|------------|------|
| Initialize | `wasm/getting_started.ts` | `rust/src/getting_started.rs` |
| Send payment | `wasm/send_payment.ts` | `rust/src/send_payment.rs` |
| Receive payment | `wasm/receive_payment.ts` | `rust/src/receive_payment.rs` |
| List payments | `wasm/list_payments.ts` | `rust/src/list_payments.rs` |
| Parse input | `wasm/parsing_inputs.ts` | `rust/src/parsing_inputs.rs` |
| LNURL-Pay | `wasm/lnurl_pay.ts` | `rust/src/lnurl_pay.rs` |
| Events | `wasm/getting_started.ts` (search `addEventListener`) | `rust/src/getting_started.rs` (search `EventListener`) |

**Minimal TypeScript example:**

```typescript
import { connect, defaultConfig } from '@breeztech/breez-sdk-spark'

const config = defaultConfig('mainnet')
config.apiKey = '<your api key>'

const sdk = await connect({
  config,
  seed: { type: 'mnemonic', mnemonic: '<12/24 words>', passphrase: undefined },
  storageDir: './.data'
})

const info = await sdk.getInfo({ ensureSynced: true })
// info.balanceSats, info.lightningAddress, info.sparkAddress

await sdk.disconnect()
```

**Minimal Rust example:**

```rust
use breez_sdk_spark::*;

let mut config = default_config(Network::Mainnet);
config.api_key = Some("<your api key>".to_string());

let sdk = connect(ConnectRequest {
    config,
    seed: Seed::Mnemonic { mnemonic: "<words>".into(), passphrase: None },
    storage_dir: "./.data".to_string(),
}).await?;

let info = sdk.get_info(GetInfoRequest { ensure_synced: Some(true) }).await?;
// info.balance_sats, info.lightning_address, info.spark_address

sdk.disconnect().await?;
```

### Core API Methods

| Method | Description |
|--------|-------------|
| `connect(config, seed, storageDir)` | Initialize SDK |
| `disconnect()` | Clean shutdown |
| `getInfo()` | Get balance, addresses, identity |
| `sendPayment(destination, amount)` | Send to any destination type |
| `receivePayment(amount, description)` | Generate Bolt11 invoice |
| `parse(input)` | Parse any input (invoice, address, LNURL) |
| `listPayments(filter)` | Get transaction history |
| `addEventListener(listener)` | Subscribe to events |

### SDK Events

- `synced` - Data synchronized, refresh UI
- `paymentSucceeded` - Payment completed
- `paymentFailed` - Payment failed
- `paymentPending` - Payment awaiting confirmation
- `claimedDeposits` - On-chain deposits claimed
- `unclaimedDeposits` - Deposits need manual claim

### Code Examples

Working code examples for all platforms are in `docs/breez-sdk/snippets/`:
- `rust/src/` - Rust examples
- `wasm/` - TypeScript/JavaScript examples
- `swift/` - Swift examples
- `kotlin_mpp_lib/` - Kotlin examples
- `flutter/lib/` - Dart examples
- `python/src/` - Python examples
- `go/` - Go examples
- `csharp/` - C# examples
- `react-native/` - React Native examples

### Full Documentation

See `docs/breez-sdk/src/guide/` for complete documentation markdown files.