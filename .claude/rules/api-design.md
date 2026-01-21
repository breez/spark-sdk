---
globs: "crates/breez-sdk/core/src/models/**/*"
---
# API Design Guidelines

Public SDK interfaces (models, enums, function signatures) require careful consideration because they're hard to change without breaking backwards compatibility.

## Roadmap Awareness

Before approving new public API additions, check `./README.md` under "SDK Development Roadmap" for planned features that might affect naming or structure.

### Current Roadmap Items (as of 2026-01)

Uncompleted items that may affect API design:

| Feature | API Design Consideration |
|---------|--------------------------|
| Bolt12 | Payment identifiers should work for both Lightning addresses and Bolt12 offers |
| NWC (Nostr Wallet Connect) | Connection/configuration models may need protocol-agnostic naming |
| WebLN | Request/response models should align with WebLN spec |
| Seedless restore | Recovery-related types should accommodate non-seed methods |
| Hodl invoices | Payment status/state should handle held payments |
| BTC <> USDX swaps | Token models need to support stablecoin tokens |

## Naming Principles

### Generic Over Specific

When multiple protocols serve similar purposes, use protocol-agnostic names:

| Avoid | Prefer | Why |
|-------|--------|-----|
| `lightning_address: String` | `payment_identifier: String` | Works for both Lightning addresses and Bolt12 offers |
| `lnurl_response: String` | `payment_info: String` | Accommodates multiple payment protocols |
| `bolt11_invoice: String` | `invoice: String` (with type field) | Allows future invoice formats |

### Extensibility Patterns

Use enums with explicit variants instead of protocol-specific fields:

```rust
// Avoid: Hard to extend
pub struct Contact {
    pub name: String,
    pub lightning_address: String,
}

// Prefer: Easy to extend
pub struct Contact {
    pub name: String,
    pub payment_method: PaymentMethod,
}

pub enum PaymentMethod {
    LightningAddress(String),
    Bolt12Offer(String),
    // Future: NWC, other protocols
}
```

## Review Checklist for Model Changes

When reviewing PRs that add or modify public models:

1. **Check roadmap** - Will planned features conflict with this naming?
2. **Consider alternatives** - Is there a more generic name that works for future protocols?
3. **Validate extensibility** - Can new variants/fields be added without breaking changes?
4. **Document rationale** - If protocol-specific naming is chosen, document why generic naming doesn't work

## Examples from Past Reviews

**PR #569 - Contacts feature:**
- Added `lightning_address: String` field
- Concern: Bolt12 is on roadmap - should this be `payment_identifier`?
- Consideration: Does the contacts feature need to support multiple payment methods per contact?
