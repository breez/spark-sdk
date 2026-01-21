---
globs: "crates/breez-sdk/bindings/**/*,packages/**/*"
---
# Binding Consistency

When changing the SDK's public interface, update all binding files together to prevent runtime mismatches.

## Synchronized Files

| Layer | File | Purpose |
|-------|------|---------|
| Core | `crates/breez-sdk/core/src/models.rs` | Add UniFFI macros to interface types |
| WASM models | `crates/breez-sdk/wasm/src/models.rs` | Export structs/enums (skip any rustdoc comments) |
| WASM interface | `crates/breez-sdk/wasm/src/sdk.rs` | WASM API surface (skip any rustdoc comments) |
| Flutter models | `packages/flutter/rust/src/models.rs` | Mirror structs/enums (skip any rustdoc comments) |
| Flutter interface | `packages/flutter/rust/src/sdk.rs` | Flutter API surface (skip any rustdoc comments) |

## Validation

CI validates binding consistency through:
- `cargo check` - Catches type mismatches at compile time
- `cargo test` - Verifies serialization compatibility
- `flutter` job - Validates Flutter binding generation
- `wasm-test` job - Tests WASM bindings in browser/Node.js

## Type Mapping

Core types map to platform-specific wrappers:

| Core | WASM | Notes |
|------|------|-------|
| `Result<T, SdkError>` | `WasmResult<T>` | Error handling |
| `Option<T>` | `Option<T>` | Direct mapping |
| `Vec<T>` | `Vec<T>` | Direct mapping |

Ensure enum variants match exactly across all targets - mismatches cause serialization failures at runtime.
