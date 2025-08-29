# Breez SDK - Nodeless *(Spark Implementation)*

## **Overview**

The Breez SDK provides developers with an end-to-end solution for integrating self-custodial Lightning into their apps and services.
It eliminates the need for third parties, simplifies the complexities of Bitcoin and Lightning, and enables seamless onboarding for billions of users to the future of value transfer.

To provide the best experience for their end-users, developers can choose between the following implementations:

- [Breez SDK -  Nodeless *(Spark Implementation)*](https://sdk-doc-spark.breez.technology/)
- [Breez SDK -  Nodeless *(Liquid Implementation)*](https://sdk-doc-liquid.breez.technology/)
- [Breez SDK - Native *(Greenlight Implementation)*](https://sdk-doc.breez.technology/)

**The Breez SDK is free for developers.**


## **What is the Breez SDK - Nodeless *(Spark Implementation)*?**

It’s a nodeless integration that offers a self-custodial, end-to-end solution for integrating Lightning, utilizing the Bitcoin-native Layer 2 Spark with on-chain interoperability. Using the SDK you'll able to:

- Send payments via various protocols such as: Bolt11, LNURL-Pay, Lightning address, BTC address, Spark address
- Receive payments via various protocols such as: Bolt11, BTC address, Spark address

**Key Features**

- [x] Send/Receive Lightning payments
- [x] Send/Receive Spark payments
- [x] On-chain interoperability
- [x] Send LNURL-pay payments
- [x] Keys are only held by users
- [x] Free open-source solution


## Getting Started 

Head over to the [Breez SDK - Nodeless *(Spark Implementation)* documentation](https://sdk-doc-spark.breez.technology/) to start implementing Lightning in your app.

You'll need an API key to use the Breez SDK - Nodeless *(Spark Implementation)*. To request an API key is free — you just need to [complete this simple form](https://breez.technology/request-api-key/#contact-us-form-sdk).


## **API**

API documentation is [here](https://breez.github.io/spark-sdk/breez_sdk_spark/index.html).


## **Command Line**

The [Breez SDK - Nodeless *(Spark Implementation)* cli](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/cli) is a command line client that allows you to interact with and test the functionality of the SDK.


## **Support**

Have a question for the team? Join our [Telegram channel](https://t.me/breezsdk) or email us at [contact@breez.technology](mailto:contact@breez.technology)
 

## How Does Nodeless *(Spark Implementation)* Work?

The Breez SDK – Nodeless (*Spark implementation*) uses Spark, a Bitcoin-native Layer 2 built on a shared signing protocol, to enable real-time, low-fee, self-custodial payments.

When sending a payment, Spark delegates the transfer of on-chain bitcoin to the recipient through a multi-signature process.
Spark Operators help facilitate the transfer, but they cannot move funds without the user. This allows the payment to settle almost instantly, without requiring a blockchain confirmation.

When receiving a payment, the same process works in reverse: the network updates ownership of the bitcoin to the user through the shared signing system, recording the change off-chain while always keeping the funds secure.

Unlike blockchains, rollups, or smart contracts, Spark doesn’t create a new ledger or require trust in external consensus.
On Bitcoin’s main chain, Spark transactions appear as a series of multi-sig wallets. Off-chain, Spark keeps a lightweight record of balances and history.

Funds are self-custodial: you can exit Spark at any time and reclaim your bitcoin directly on the Bitcoin main chain.


## **Build & Test**

- **crates**: Contains the root Rust cargo workspace.
    - **breez-sdk**: Collection of Breez SDK crates.
        - **bindings**: The FFI bindings for Kotlin, Flutter, Python, React Native, and Swift.
        - **cli**: Contains the Rust command line interface client for the SDK.
        - **core**: The core Breez SDK Rust library.
    - **spark**: The Spark crate.
- **packages**: Contains the packages for WASM.


## **Contributing**

Contributions are always welcome. Please read our [contribution guide](CONTRIBUTING.md) to get started.


## **SDK Development Roadmap**

- [x]  Send/Receive Lightning payments
- [x]  Send/Receive Spark payments
- [x]  CLI Interface
- [x]  Foreign languages bindings
- [x]  Pay BTC on-chain
- [x]  Receive via on-chain address
- [x]  Send LNURL-Pay
- [x]  Send to a Lightning address
- [x]  WebAssembly
- [ ]  Support Spark tokens
- [ ]  Fiat on-ramp
- [ ]  Webhook for receiving payments
- [ ]  Export/Import SDK data
- [ ]  Offline receive via notifications
- [ ]  Real-time sync
- [ ]  External input parsers
- [ ]  BIP353 pay codes
- [ ]  Receive LNURL-Pay
- [ ]  LNURL-Withdraw
- [ ]  LNURL-Auth
- [ ]  Receive via Lightning address
- [ ]  Bolt12 
- [ ]  WebLN
- [ ]  NWC
- [ ]  Add fees via a dedicated portal



