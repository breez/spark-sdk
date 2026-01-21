---
globs: "{crates,packages/flutter/rust}/**/*.rs"
---
# Rust Conventions

## Error Handling

Use the crate's error types (`SdkError`, `SparkError`) for public APIs. Provide context with `.context()` or custom error variants so callers understand what failed.

```rust
// Preferred: Returns error with context
pub fn process(&self) -> Result<Output, SdkError> {
    let data = self.fetch().context("fetching data for processing")?;
    Ok(transform(data))
}
```

Reserve `unwrap()` and `expect()` for tests and CLI tools where panicking is acceptable.

## Type Design

Derive UniFFI macros for types exposed to language bindings:

```rust
#[derive(Debug, Clone)]
#[cfg_attr(feature = "uniffi", derive(uniffi::Record))]
pub struct PaymentRequest {
    pub amount_msat: u64,
    pub description: String,
}

#[cfg_attr(feature = "uniffi", derive(uniffi::Enum))]
pub enum PaymentStatus {
    Pending,
    Complete,
    Failed,
}
```

Add serde derives for types that need persistence:

```rust
#[derive(Serialize, Deserialize)]
pub struct PersistedState { /* ... */ }
```

Implement `From`/`Into` for type conversions between layers.

## Async Patterns

The workspace uses tokio as the async runtime. Prefer async functions over spawning tasks when the caller can await the result directly.

## Testing

- Place unit tests in the same file with `#[cfg(test)]` module
- Use `*-itest` crates for integration tests requiring external services
- Test error paths and edge cases, not just success scenarios
