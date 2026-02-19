# Receiving payments

The SDK provides dedicated methods for each way to receive payments. Choose the
method that matches your use case:

| Method | What you get | Use case |
|--------|-------------|----------|
| {{#name create_invoice}} | BOLT11 Lightning invoice | Receive via Lightning Network |
| {{#name create_spark_invoice}} | Spark invoice | Receive via Spark (supports tokens) |
| {{#name get_bitcoin_address}} | Bitcoin deposit address | Receive on-chain Bitcoin |
| {{#name get_spark_address}} | Spark address | Receive via Spark address |

## Creating a Lightning invoice

Use {{#name create_invoice}} to generate a BOLT11 invoice:

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,no_run
{{#include ../../snippets/rust/src/receive.rs:create-invoice}}
```

</section>
<div slot="title">TypeScript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive.ts:create-invoice}}
```

</section>
</custom-tabs>

## Creating a Spark invoice

Use {{#name create_spark_invoice}} to generate a Spark invoice. Spark invoices
can also receive token payments by specifying a `token_identifier`:

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,no_run
{{#include ../../snippets/rust/src/receive.rs:create-spark-invoice}}
```

</section>
<div slot="title">TypeScript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive.ts:create-spark-invoice}}
```

</section>
</custom-tabs>

## Getting a Bitcoin deposit address

Use {{#name get_bitcoin_address}} to obtain an on-chain deposit address:

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,no_run
{{#include ../../snippets/rust/src/receive.rs:get-bitcoin-address}}
```

</section>
<div slot="title">TypeScript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive.ts:get-bitcoin-address}}
```

</section>
</custom-tabs>

## Getting a Spark address

Use {{#name get_spark_address}} to obtain the wallet's Spark address:

<custom-tabs category="lang">
<div slot="title">Rust</div>
<section>

```rust,no_run
{{#include ../../snippets/rust/src/receive.rs:get-spark-address}}
```

</section>
<div slot="title">TypeScript</div>
<section>

```typescript
{{#include ../../snippets/wasm/receive.ts:get-spark-address}}
```

</section>
</custom-tabs>
