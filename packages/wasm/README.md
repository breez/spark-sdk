# Breez SDK - Nodeless (_Spark Implementation_)

## **What Is the Breez SDK?**

The Breez SDK provides developers with an end-to-end solution for integrating self-custodial Lightning into their apps and services. It eliminates the need for third parties, simplifies the complexities of Bitcoin and Lightning, and enables seamless onboarding for billions of users to the future of value transfer.

## **What Is the Breez SDK - Nodeless _(Spark Implementation)_?**

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

Head over to the Breez SDK - Nodeless _(Spark Implementation)_ [documentation](https://sdk-doc-spark.breez.technology/) to start implementing Lightning in your app.

### Web

When developing a browser application you should import `@breeztech/breez-sdk-spark` (or the explicit `@breeztech/breez-sdk-spark/web` submodule).

It's important to first initialise the WebAssembly module by using `await init()` before making any other calls to the module.

```js
import init, {
  initLogging,
  defaultConfig,
  SdkBuilder,
} from "@breeztech/breez-sdk-spark/web";

// Initialise the WebAssembly module
await init();
```

### Node.js

> **Note**: This package requires Node.js v22 or higher.

When developing a node.js application you should require `@breeztech/breez-sdk-spark` (or the explicit `@breeztech/breez-sdk-spark/node` submodule).

```js
const {
  initLogging,
  defaultConfig,
  SdkBuilder,
} = require("@breeztech/breez-sdk-spark/nodejs");
const { Command } = require("commander");
require("dotenv").config();

class JsLogger {
  log = (logEntry) => {
    console.log(
      `[${new Date().toISOString()} ${logEntry.level}]: ${logEntry.line}`
    );
  };
}

const fileLogger = new JsLogger();

class JsEventListener {
  onEvent = (event) => {
    fileLogger.log({
      level: "INFO",
      line: `Received event: ${JSON.stringify(event)}`,
    });
  };
}

const eventListener = new JsEventListener();
const program = new Command();

const initSdk = async () => {
  // Set the logger to trace
  await initLogging(fileLogger);

  // Get the mnemonic
  const mnemonic = process.env.MNEMONIC;

  // Connect using the config
  let config = defaultConfig("regtest");
  config.apiKey = process.env.BREEZ_API_KEY;
  console.log(`defaultConfig: ${JSON.stringify(config)}`);

  let sdkBuilder = SdkBuilder.new(config, {
    type: "mnemonic",
    mnemonic: mnemonic,
  });
  sdkBuilder = await sdkBuilder.withDefaultStorage("./.data");

  const sdk = await sdkBuilder.build();

  await sdk.addEventListener(eventListener);
  return sdk;
};

program
  .name("breez-sdk-spark-wasm-cli")
  .description("CLI for Breez SDK Spark - Wasm");

program.command("get-info").action(async () => {
  const sdk = await initSdk();
  const res = await sdk.getInfo({});
  console.log(`getInfo: ${JSON.stringify(res)}`);
});

program.parse();
```

## Pricing

The Breez SDK is **free** for developers.

## Support

Have a question for the team? Join us on [Telegram](https://t.me/breezsdk) or email us at <contact@breez.technology>.
