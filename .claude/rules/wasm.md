---
globs: "**/wasm/**/*"
---
# WASM Guidelines

## Build and Test

| Task | Command |
|------|---------|
| Build | `make build-wasm` |
| Test | `make wasm-test` |

Tests run in both browser and Node.js environments.

## API Design

Keep the WASM API surface minimal - each export increases bundle size. Create WASM-specific DTOs when internal types are too complex or expose implementation details.

Use `WasmResult<T>` for error handling:

```rust
#[wasm_bindgen]
pub async fn connect(config: WasmConnectConfig) -> WasmResult<WasmBreezSdk> {
    // Implementation
}
```

## Async

WASM uses `wasm-bindgen-futures` for async operations. Long-running operations should support cancellation where feasible since users may navigate away from the page.

## Type Conversions

Convert between core and WASM types at the boundary:

```rust
impl From<CorePayment> for WasmPayment {
    fn from(p: CorePayment) -> Self {
        WasmPayment {
            // Map fields
        }
    }
}
```
