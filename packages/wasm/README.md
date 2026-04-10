# Breez SDK - Spark

## **What is the Breez SDK?**

The Breez SDK provides developers with an end-to-end solution for integrating self-custodial Lightning into their apps and services. It eliminates the need for third parties, simplifies the complexities of Bitcoin and Lightning, and enables seamless onboarding for billions of users to the future of value transfer.

## **What is the Breez SDK - Spark?**

It’s a nodeless integration that offers a self-custodial, end-to-end solution for integrating Lightning payments, utilizing Spark with on-chain interoperability and third-party fiat on-ramps.

## Installation

To install the package:

```sh
npm install @breeztech/breez-sdk-spark
```

or

```sh
yarn add @breeztech/breez-sdk-spark
```

## Usage

Head over to the Breez SDK - Spark [documentation](https://sdk-doc-spark.breez.technology/) to start implementing Lightning in your app.

### Web

When developing a browser application, import from `@breeztech/breez-sdk-spark` (or the explicit `@breeztech/breez-sdk-spark/web` subpath).

Call `await init()` to load the WebAssembly module before using any other SDK methods.

```js
import init, {
  defaultConfig,
  connect,
} from "@breeztech/breez-sdk-spark/web";

await init();

const config = defaultConfig("mainnet");
config.apiKey = "<your api key>";

const sdk = await connect({
  config,
  seed: { type: "mnemonic", mnemonic: "<words>", passphrase: undefined },
  storageDir: "./.data",
});
```

### SSR Frameworks (Next.js, SvelteKit, Nuxt, Remix)

Use the `@breeztech/breez-sdk-spark/ssr` subpath in SSR applications. It can be imported during server-side rendering without errors — no WASM is loaded, no browser or Node.js APIs are touched. Call `init()` on the client to load the WASM module before using SDK functions.

```tsx
"use client";
import { useEffect, useState } from "react";
import init, { connect, defaultConfig } from "@breeztech/breez-sdk-spark/ssr";

export default function Wallet() {
  const [sdk, setSdk] = useState(null);

  useEffect(() => {
    (async () => {
      await init(); // Loads WASM — client-side only
      const config = defaultConfig("mainnet");
      config.apiKey = "<your api key>";
      const s = await connect({
        config,
        seed: { type: "mnemonic", mnemonic: "<words>", passphrase: undefined },
        storageDir: "./.data",
      });
      setSdk(s);
    })();
  }, []);

  return <div>{sdk ? "Connected" : "Loading..."}</div>;
}
```

### Node.js

> **Note**: This package requires Node.js v22 or higher.

When developing a Node.js application, use `require("@breeztech/breez-sdk-spark")` (or the explicit `@breeztech/breez-sdk-spark/nodejs` subpath). No `init()` call is needed — the WASM module loads automatically.

```js
const {
  defaultConfig,
  connect,
} = require("@breeztech/breez-sdk-spark");

const config = defaultConfig("mainnet");
config.apiKey = process.env.BREEZ_API_KEY;

const sdk = await connect({
  config,
  seed: { type: "mnemonic", mnemonic: process.env.MNEMONIC, passphrase: undefined },
  storageDir: "./.data",
});

const info = await sdk.getInfo({});
console.log(`Balance: ${info.balanceSats} sats`);
```

### Subpath Exports

| Subpath | Environment | Module Format |
|---------|-------------|---------------|
| `@breeztech/breez-sdk-spark` | Node.js / Browser (default) | CJS / ESM |
| `@breeztech/breez-sdk-spark/web` | Browser | ESM |
| `@breeztech/breez-sdk-spark/nodejs` | Node.js | CJS |
| `@breeztech/breez-sdk-spark/bundler` | Bundler (Webpack, Vite) | ESM |
| `@breeztech/breez-sdk-spark/deno` | Deno | ESM |
| `@breeztech/breez-sdk-spark/ssr` | SSR (explicit) | ESM |

## Pricing

The Breez SDK is **free** for developers.

## Support

Have a question for the team? Join us on [Telegram](https://t.me/breezsdk) or email us at <contact@breez.technology>.
