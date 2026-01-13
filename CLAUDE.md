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
2. **crates/breez-sdk/wasm/src/models.rs** - Update exported structs/enums (skip any rustdoc comments)
3. **crates/breez-sdk/wasm/src/sdk.rs** - Update WASM interface (skip any rustdoc comments)
4. **packages/flutter/rust/src/models.rs** - Update mirrored structs/enums (skip any rustdoc comments)
5. **packages/flutter/rust/src/sdk.rs** - Update Flutter interface (skip any rustdoc comments)

## Workspace Configuration

- Rust edition 2024, MSRV 1.88
- Clippy: pedantic + suspicious + complexity + perf warnings enabled
- Release builds use LTO and `opt-level = "z"` for size optimization
- Uses `cargo xtask` for build automation (aliased in `.cargo/config.toml`)
