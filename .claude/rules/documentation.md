---
globs: "docs/**/*,**/*.md"
---
# Documentation Guidelines

## Code Snippets

Maintain parallel examples in all 9 supported languages:

| Language | Location |
|----------|----------|
| Rust | `docs/breez-sdk/snippets/rust/` |
| Go | `docs/breez-sdk/snippets/go/` |
| Python | `docs/breez-sdk/snippets/python/` |
| Kotlin | `docs/breez-sdk/snippets/kotlin_mpp_lib/` |
| Swift | `docs/breez-sdk/snippets/swift/` |
| C# | `docs/breez-sdk/snippets/csharp/` |
| Flutter | `docs/breez-sdk/snippets/flutter/` |
| WASM | `docs/breez-sdk/snippets/wasm/` |
| React Native | `docs/breez-sdk/snippets/react-native/` |

When adding a new example, add it to all languages to keep documentation synchronized.

## ANCHOR Markers

Use paired markers for mdbook extraction:

```rust
// ANCHOR: send_payment
pub async fn send_payment(&self, req: SendPaymentRequest) -> Result<Payment> {
    // ...
}
// ANCHOR_END: send_payment
```

Every `ANCHOR:` marker needs a matching `ANCHOR_END:` marker.

## API Documentation

Public functions need doc comments explaining:
- What the function does (one sentence summary)
- Parameters and their constraints
- Return value and error conditions
- Example usage for complex APIs
