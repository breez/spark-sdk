---
globs: "**/signer/**/*,**/token/**/*,crates/spark-wallet/**/*"
---
# Security Guidelines

## Threat Model Approach

When reviewing security-sensitive code, think about threats rather than checking boxes. Consider what could go wrong and how the code defends against it.

## Trust Boundaries

Data crossing these boundaries requires validation:

| Boundary | Source | Considerations |
|----------|--------|----------------|
| LNURL responses | External servers | Validate all fields, handle malformed data gracefully |
| Spark operator messages | Federated operators | Verify signatures, check for replay attacks |
| Swap providers | Third-party services | Validate amounts, check for fee manipulation |
| User input | App layer | Validate addresses, amounts, and identifiers |

## Boundary Crossing Checklist

For each trust boundary crossing:
1. Is input validated before use?
2. What happens with malformed or unexpected data?
3. Can the operation be replayed to cause harm?
4. Are error messages safe to expose to callers?

## State and Recovery

Design for crash safety:
- Persist state before making external calls when possible
- Handle partial completion states on restart
- Make operations idempotent where feasible

## Secret Handling

Keep secrets out of observable channels:
- Log messages should exclude keys, seeds, and authentication tokens
- Error messages should describe failures without exposing sensitive data
- Zero secrets in memory after use when performance allows

## Numeric Safety

For monetary calculations:
- Use checked arithmetic (`checked_add`, `checked_mul`) to catch overflow
- Use integer satoshis/millisatoshis for amounts at protocol boundaries (floating-point introduces rounding errors)
- Validate ranges before type conversions (e.g., `u64` to `i64`)
