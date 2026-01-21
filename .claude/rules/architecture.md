# Architecture

## Crate Structure

- **crates/breez-sdk/core** - Main SDK library with public API (`BreezSdk`)
- **crates/breez-sdk/common** - Shared utilities, LNURL support, networking, sync protocol
- **crates/breez-sdk/bindings** - UniFFI bindings for Go, Kotlin, Python, React Native, Swift
- **crates/breez-sdk/wasm** - WebAssembly bindings for JavaScript/TypeScript
- **crates/breez-sdk/cli** - Command-line interface for testing
- **crates/spark** - Low-level Spark protocol (addresses, signing, operators, tokens)
- **crates/spark-wallet** - High-level wallet operations wrapping Spark protocol
- **crates/xtask** - Custom build tasks (powers `make` commands via `cargo xtask`)

## Key Abstractions

- `Storage` trait - Pluggable persistence layer (default: SQLite)
- `Signer` trait - Cryptographic operations (FROST threshold signing)
- `BitcoinChainService` trait - Blockchain provider interface
- `EventEmitter` - Broadcasts `SdkEvent` (Synced, PaymentSucceeded, PaymentFailed, etc.)

## Data Flow

```
BreezSdk (API) → SparkWallet (wallet ops) → Spark (protocol) → Operators (gRPC)
     ↓
Storage → SyncedStorage → Breez Sync Service (multi-device)
```

## Cargo Workspace

Configuration for the Rust workspace (`Cargo.toml` at repo root):

- Rust edition 2024, MSRV 1.88
- Clippy: pedantic + suspicious + complexity + perf warnings enabled
- Release builds use LTO and `opt-level = "z"` for size optimization
- Build automation via `cargo xtask` (aliased in `.cargo/config.toml`)
