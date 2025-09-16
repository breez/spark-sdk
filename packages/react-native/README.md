# Breez SDK - Nodeless (*Spark Implementation*)

## **What Is the Breez SDK?**

The Breez SDK provides developers with an end-to-end solution for integrating self-custodial Lightning into their apps and services. It eliminates the need for third parties, simplifies the complexities of Bitcoin and Lightning, and enables seamless onboarding for billions of users to the future of value transfer.

## **What Is the Breez SDK - Nodeless *(Spark Implementation)*?**

It’s a nodeless integration that offers a self-custodial, end-to-end solution for integrating Lightning payments, utilizing Spark with on-chain interoperability and third-party fiat on-ramps.

## Installation

To install the package:

```sh
npm install @breeztech/breez-sdk-spark-react-native
```

## Usage

Head over to the Breez SDK - Nodeless *(Spark Implementation)* [documentation](https://sdk-doc-spark.breez.technology/) to start implementing Lightning in your app.

```js
import { connect, defaultConfig } from '@breeztech/breez-sdk-spark-react-native';
import RNFS from 'react-native-fs';

// ...

const mnemonic = 
  'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about';

let config = defaultConfig(Network.Mainnet);
config.apiKey = apiKey;

const sdk = await connect({
  config,
  mnemonic,
  storageDir: `${RNFS.DocumentDirectoryPath}/breezSdkSpark`,
});
```

## Pricing

The Breez SDK is **free** for developers. 

## Support

Have a question for the team? Join us on [Telegram](https://t.me/breezsdk) or email us at <contact@breez.technology>.

## Information for Maintainers and Contributors

This repository is used to publish a NPM package providing React Native bindings to the Breez SDK - Nodeless *(Spark Implementation)*'s [underlying Rust implementation](https://github.com/breez/spark-sdk). The React Native bindings are generated using [UniFFi Bindgen React Native](https://github.com/jhugman/uniffi-bindgen-react-native).

Any changes to Breez SDK - Nodeless *(Spark Implementation)*, the React Native bindings, and the configuration of this React Native package must be made via the [spark-sdk](https://github.com/breez/spark-sdk) repository.
