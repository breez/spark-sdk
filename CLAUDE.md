# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Overview

This is the **Breez SDK - Nodeless (Spark Implementation)**, a self-custodial Lightning SDK that uses Spark, a Bitcoin-native Layer 2 built on a shared signing protocol. The SDK enables sending and receiving Lightning payments, Spark payments, and on-chain Bitcoin transactions without running a Lightning node.

## Build Commands

### Basic Build & Development

```bash
# Build the project
make build

# Build for WebAssembly
make build-wasm

# Build for a specific target (e.g., wasm32-unknown-unknown)
make build-target-wasm32-unknown-unknown

# Build release version
make build-release
```

### Testing

```bash
# Run all tests (cargo + wasm)
make test

# Run only cargo tests
make cargo-test

# Run only wasm tests (browser)
make wasm-test-browser

# Run wasm tests in Node.js
make wasm-test-node

# Run integration tests (requires Docker)
make itest
```

**Note:** Integration tests (`make itest`) require Docker to be running. They automatically pull required images (bitcoind, postgres) and build local images for Spark operators.

### Linting & Formatting

```bash
# Check formatting and run clippy
make check

# Format code
make fmt-fix

# Check formatting without modifying
make fmt-check

# Run clippy
make clippy-check

# Auto-fix clippy issues
make clippy-fix

# Run clippy for wasm target
make wasm-clippy-check
```

### Running Specific Tests

```bash
# Test a specific package
cargo xtask test -p breez-sdk-spark

# Run doctests only
cargo xtask test --doc

# Test a specific package with extra args
cargo xtask test -p spark-wallet -- --nocapture
```

## Architecture

### Workspace Structure

The project is organized as a Cargo workspace with the following key crates:

- **`crates/breez-sdk/`**: Main SDK implementation
  - **`core/`**: Core SDK logic (`breez-sdk-spark` crate)
  - **`bindings/`**: FFI bindings for Go, Kotlin, Python, React Native, Swift (uses UniFFI)
  - **`cli/`**: Command-line interface for testing SDK functionality
  - **`common/`**: Shared utilities and types
  - **`wasm/`**: WebAssembly bindings
  - **`breez-itest/`**: Integration tests for Breez SDK

- **`crates/spark/`**: Low-level Spark protocol implementation
  - Handles FROST multi-signature operations
  - Manages session state and operator communication
  - Provides core Spark transfer functionality

- **`crates/spark-wallet/`**: Wallet layer on top of Spark
  - Wraps `spark` crate with wallet-specific logic
  - Manages Lightning invoice handling, on-chain withdrawals, deposits
  - Provides higher-level transfer APIs

- **`crates/spark-itest/`**: Integration tests for Spark protocol
  - Requires Docker containers (bitcoind, postgres, Spark operators)

- **`crates/xtask/`**: Build automation tool (invoked via `cargo xtask`)

- **`packages/`**: Language-specific packages for Flutter, React Native, Wasm

### Key Architectural Layers

1. **SDK Layer** (`breez-sdk/core`):
   - Entry point: `BreezSdk` struct in `sdk.rs`
   - Provides request/response API (e.g., `SendPaymentRequest` → `SendPaymentResponse`)
   - Handles event emission, storage persistence, background syncing
   - Orchestrates deposits, withdrawals, and LNURL operations
   - Validates Breez API key (required for non-regtest networks)

2. **Wallet Layer** (`spark-wallet`):
   - Entry point: `SparkWallet` struct
   - Manages Spark transfers, Lightning invoices, on-chain operations
   - Uses `spark` crate for low-level protocol operations
   - Handles operator pool configuration and fee quotes

3. **Spark Protocol Layer** (`spark`):
   - Implements FROST threshold signatures for shared signing
   - Manages session state with Spark operators
   - Handles gRPC/tonic communication with operators
   - Core types: `SessionManager`, `Signer`, `Operator`, `Tree`

4. **Storage Layer** (`persist` module in `core`):
   - `Storage` trait defines persistence interface
   - `SqliteStorage` implementation (non-WASM)
   - Stores payments, deposits, cached data (balance, sync info, lightning address)

5. **Chain Services** (`chain` module in `core`):
   - `BitcoinChainService` trait for on-chain operations
   - `RestClientChainService` implementation
   - Fetches UTXOs, broadcasts transactions

### Multi-Platform Support

The SDK supports multiple platforms through conditional compilation:

- **Native (not WASM)**: Uses `tokio` with multi-threaded runtime, `rusqlite` for storage
- **WASM**: Uses `tokio_with_wasm`, in-memory or IndexedDB storage, `tonic-web-wasm-client`

WASM-specific packages are excluded from normal builds (see `workspace_exclude_wasm()` in `xtask`).

### Background Tasks

The SDK starts background tasks on initialization (`BreezSdk::start()`):

1. **Periodic Sync**: Polls Spark network at `sync_interval_secs` (default 60s)
   - Triggered automatically or via `sync_wallet()` API
   - Syncs payments, checks for deposits, claims deposits automatically
   - Uses `sync_trigger` broadcast channel for coordination

2. **Deposit Monitoring**: Watches for on-chain deposits and auto-claims them
   - `DepositChainSyncer` checks UTXOs against static deposit addresses
   - Respects `max_deposit_claim_fee` config

3. **Lightning Address Recovery**: Attempts to recover lightning address on startup

### Payment Flow

**Sending a payment:**
1. `prepare_send_payment()`: Parse input (Spark address, Bolt11, Bitcoin address), fetch fees
2. `send_payment()`: Execute transfer via `SparkWallet`
3. For Lightning: May poll SSP for payment status updates
4. Emit `PaymentSucceeded` or `PaymentFailed` event

**Receiving a payment:**
1. `receive_payment()`: Generate Spark address, Bitcoin address, or Bolt11 invoice
2. Background sync detects incoming transfer
3. Auto-claim if deposit, emit `PaymentSucceeded` event

### UniFFI Bindings

When changing SDK interfaces:
- Add `#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]` to structs
- Add `#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]` to enums
- Update bindings in `crates/breez-sdk/bindings/`, `crates/breez-sdk/wasm/`, `packages/flutter/rust/`

### Testing Strategy

- **Unit tests**: In each crate (run with `cargo xtask test`)
- **WASM tests**: Use `wasm-bindgen-test`, run in browser or Node.js
- **Integration tests**: `spark-itest` for Spark protocol, `breez-itest` for SDK (require Docker)
- Use `#[cfg(feature = "test-utils")]` for test-only code (e.g., `claim_deposit_with_tx()`)

## Development Notes

### API Key Requirement

The SDK requires a valid Breez API key for mainnet/testnet. In regtest mode, the API key is optional. Validation happens in `validate_breez_api_key()` which checks the certificate issuer.

### Rust Version

Minimum supported Rust version: **1.88** (see `workspace.package.rust-version` in root `Cargo.toml`)

### Clippy Configuration

The workspace enforces strict clippy lints (`-D warnings`). Some lints are allowed:
- `missing_errors_doc`, `missing_panics_doc`, `must_use_candidate`, `struct_field_names`
- `arithmetic_side_effects` is set to `warn`

### macOS WASM Builds

On macOS, `xtask` auto-detects Homebrew LLVM for cross-compiling to WASM (sets `CC_wasm32_unknown_unknown` and `AR_wasm32_unknown_unknown`).

### LNURL Support

LNURL-pay is implemented with optional lightning address support:
- Configure `lnurl_domain` in `Config` (defaults to "breez.tips")
- `lnurl_server_client` handles registration/recovery of lightning addresses
- See `crates/breez-sdk/core/src/lnurl/` for implementation

### Prefer Spark vs Lightning

The `prefer_spark_over_lightning` config flag determines payment routing:
- `true`: Use Spark transfers when possible (lower fees, less privacy)
- `false` (default): Use Lightning network when possible

### Common Patterns

- **Request/Response objects**: All SDK methods take typed request objects and return response objects
- **Event emission**: Use `EventEmitter` to notify listeners of SDK events (`Synced`, `PaymentSucceeded`, etc.)
- **Error handling**: SDK errors are wrapped in `SdkError` enum
- **Async/await**: All async code uses `tokio` (or `tokio_with_wasm` for WASM)

### CLI Tool

Use `crates/breez-sdk/cli` to manually test SDK functionality during development. Build with:

```bash
cargo build -p breez-sdk-cli
```
