# Parsing inputs (Action-based)

The SDK can parse various payment input strings and return a structured action
indicating what the user can do with the parsed input.

Use {{#name parse_action}} to parse an input and receive a {{#enum ParsedAction}}
that categorises the result:

| Action | Description |
|--------|-------------|
| **Send** | The input is a payment destination (invoice, address, LNURL-Pay, etc.) |
| **Receive** | The input allows receiving funds (e.g. LNURL-Withdraw) |
| **Authenticate** | The input is an LNURL-Auth challenge |
| **Multi** | The input contains multiple payment methods (e.g. BIP21 URI) |
| **Unsupported** | The input was parsed but is not directly actionable |

## Parsing and handling actions

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,no_run
{{#include ../../snippets/rust/src/parse_action.rs:parse-action}}
```

</section>
<div slot="title">TypeScript</div>
<section>

```typescript
{{#include ../../snippets/wasm/parse_action.ts:parse-action}}
```

</section>
</custom-tabs>

## Static parsing (no SDK instance)

You can also parse inputs without an SDK connection using the
{{#name parse_action}} free function:

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,no_run
{{#include ../../snippets/rust/src/parse_action.rs:parse-action-static}}
```

</section>
<div slot="title">TypeScript</div>
<section>

```typescript
{{#include ../../snippets/wasm/parse_action.ts:parse-action-static}}
```

</section>
</custom-tabs>

## Action types

### Send

A {{#enum ParsedAction::Send}} wraps a {{#name SendAction}} enum with these variants:

| Variant | Data | Notes |
|---------|------|-------|
| `Bolt11` | `Bolt11InvoiceDetails` | Standard Lightning invoice |
| `Bolt12Invoice` | `Bolt12InvoiceDetails` | BOLT12 invoice |
| `Bolt12Offer` | `Bolt12OfferDetails` | BOLT12 offer |
| `SparkInvoice` | `SparkInvoiceDetails` | Spark protocol invoice |
| `SparkAddress` | `SparkAddressDetails` | Spark address |
| `Bitcoin` | `BitcoinAddressDetails` | On-chain Bitcoin address |
| `LnurlPay` | `LnurlPayRequestDetails` | LNURL-Pay endpoint |
| `LightningAddress` | `LightningAddressDetails` | Lightning address (user@domain) |

Use {{#name prepare_send}} to prepare a payment from any send action, then
{{#name send_payment}} to execute it.

### Receive

A {{#enum ParsedAction::Receive}} wraps a {{#name ReceiveAction}} enum:

| Variant | Data | Notes |
|---------|------|-------|
| `LnurlWithdraw` | `LnurlWithdrawRequestDetails` | LNURL-Withdraw endpoint |

Use {{#name withdraw}} to execute the withdrawal.

### Authenticate

A {{#enum ParsedAction::Authenticate}} contains an {{#name AuthAction}} with the
LNURL-Auth challenge details. Use {{#name authenticate}} to complete the
authentication.

### Multi

A {{#enum ParsedAction::Multi}} is returned for BIP21 URIs that contain multiple
payment methods. It includes:

- `bip21_details` — the original BIP21 metadata (amount, label, etc.)
- `actions` — a list of individual {{#name ParsedAction}} entries

Your app should present the available options and let the user (or your logic)
pick the preferred method.
