# Breez SDK - Nodeless (_Spark Implementation_)

## **What Is the Breez SDK?**

The Breez SDK provides developers with an end-to-end solution for integrating self-custodial Lightning into their apps and services. It eliminates the need for third parties, simplifies the complexities of Bitcoin and Lightning, and enables seamless onboarding for billions of users to the future of value transfer.

## **What Is the Breez SDK - Nodeless _(Spark Implementation)_?**

It's a nodeless integration that offers a self-custodial, end-to-end solution for integrating Lightning payments, utilizing Spark with on-chain interoperability and third-party fiat on-ramps.

## Installation

```sh
npm install @breeztech/breez-sdk-spark
```

## Quick Start

### Web

When developing a browser application, first initialize the WebAssembly module before making any other SDK calls:

```ts
import init, { Breez } from "@breeztech/breez-sdk-spark/web";

await init();

const client = await Breez.connect({
  apiKey: "<breez api key>",
  network: "mainnet",
  seed: { type: "mnemonic", mnemonic: "<mnemonic words>", passphrase: undefined },
});

const info = await client.getInfo({ ensureSynced: true });
console.log(`Balance: ${info.balanceSats} sats`);

await client.disconnect();
```

### Node.js

> **Note**: Requires Node.js v22 or higher.

```ts
const { Breez, initLogging } = require("@breeztech/breez-sdk-spark/nodejs");

// Optional: enable logging
await initLogging({
  log: (entry) => console.log(`[${entry.level}]: ${entry.line}`),
});

const client = await Breez.connect({
  apiKey: "<breez api key>",
  network: "mainnet",
  seed: { type: "mnemonic", mnemonic: "<mnemonic words>", passphrase: undefined },
  storageDir: "./.data",
});

const info = await client.getInfo({ ensureSynced: true });
console.log(`Balance: ${info.balanceSats} sats`);

await client.disconnect();
```

## API Overview

### Entry Points

| Method | Description |
|--------|-------------|
| `Breez.connect(config)` | Single-step connection (most common) |
| `new Breez(appConfig)` | Multi-client setup |
| `breez.connectClient(clientConfig)` | Connect additional clients |

### BreezClient — Core Methods

| Method | Description |
|--------|-------------|
| `client.getInfo(request?)` | Get balance and identity pubkey |
| `client.preparePayment(destination, options?)` | Prepare a payment (inspect fees before sending) |
| `client.sendPayment(destination, prepareOpts?, payOpts?)` | One-step send (no review) |
| `client.receive(options)` | Generate invoice, BTC address, or Spark address |
| `client.disconnect()` | Clean shutdown |
| `client.pubkey` | Identity public key (sync, no network call) |

### BreezClient — Sub-Object APIs

Access grouped functionality through property getters:

```ts
client.payments         // PaymentsApi
client.deposits         // DepositsApi
client.events           // EventsApi
client.lightningAddress // LightningAddressApi
client.lnurl            // LnurlApi
client.fiat             // FiatApi
client.settings         // SettingsApi
client.message          // MessageApi
client.tokens           // TokensApi
client.optimization     // OptimizationApi
client.tokenIssuer      // TokenIssuer
```

| API | Methods |
|-----|---------|
| **PaymentsApi** | `list(limit?, offset?)`, `get(id)` |
| **DepositsApi** | `listUnclaimed()`, `claim(request)`, `refund(request)` |
| **EventsApi** | `add(listener)`, `remove(id)`, `on(eventType, callback)` |
| **LightningAddressApi** | `get()`, `register(request)`, `isAvailable(username)`, `delete()` |
| **LnurlApi** | `auth(requestData)`, `withdraw(request)` |
| **FiatApi** | `rates()`, `currencies()` |
| **SettingsApi** | `get()`, `update(request)` |
| **MessageApi** | `sign(request)` |
| **TokensApi** | `metadata(request)`, `swapLimits(request)` |

### Payment Intent Flow (Recommended)

The two-step `preparePayment` + `send` flow lets users review fees before committing:

```ts
// 1. Prepare the payment (nothing is sent yet)
const payment = await client.preparePayment("<bolt11 invoice or address>");

// 2. Inspect fees
console.log(`Amount: ${payment.amount} sats`);
console.log(`Fee: ${payment.feeSats} sats`);
console.log(`Type: ${payment.paymentType}`);

// 3. Send when ready
const result = await payment.send();
console.log(`Payment ID: ${result.payment.id}`);
```

### Receiving Payments

```ts
// Lightning invoice
const bolt11 = await client.receive({
  paymentMethod: {
    type: "bolt11Invoice",
    amountSats: 5000,
    description: "Coffee",
  },
});

// On-chain Bitcoin address
const btc = await client.receive({
  paymentMethod: { type: "bitcoinAddress" },
});

// Spark address
const spark = await client.receive({
  paymentMethod: { type: "sparkAddress" },
});
```

### Events

```ts
const listenerId = await client.events.add({
  onEvent: (event) => {
    switch (event.type) {
      case "synced":
        // Refresh UI - balance and payments are up to date
        break;
      case "paymentSucceeded":
        console.log("Payment completed:", event.payment.id);
        break;
      case "paymentFailed":
        console.log("Payment failed:", event.payment.id);
        break;
    }
  },
});

// Remove when done
await client.events.remove(listenerId);
```

### Advanced: SdkBuilder

For custom storage, chain services, or external signers:

```ts
import { SdkBuilder, defaultConfig } from "@breeztech/breez-sdk-spark";

const config = defaultConfig("mainnet");
config.apiKey = "<breez api key>";

let builder = SdkBuilder.new(config, {
  type: "mnemonic",
  mnemonic: "<mnemonic words>",
});
builder = await builder.withDefaultStorage("./.data");
// builder = builder.withStorage(customStorage);
// builder = builder.withChainService(customChainService);

const client = await builder.build();
```

## Networks

| Network | Use Case |
|---------|----------|
| `mainnet` | Production |
| `testnet` | Testing with testnet Bitcoin |
| `regtest` | Development (no API key needed) |

**Regtest** is recommended for development — free, no real value, supports all payment types.

## Code Examples

Working, compiled TypeScript examples for every feature are in [`docs/breez-sdk/snippets/wasm/`](../../docs/breez-sdk/snippets/wasm/).

## Full Documentation

[sdk-doc-spark.breez.technology](https://sdk-doc-spark.breez.technology/)

## Pricing

The Breez SDK is **free** for developers.

## Support

Have a question for the team? Join us on [Telegram](https://t.me/breezsdk) or email us at <contact@breez.technology>.
