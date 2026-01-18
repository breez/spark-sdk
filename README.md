# Breez SDK - Nodeless *(Spark Implementation)*

## **Overview**

The Breez SDK provides developers with an end-to-end solution for integrating self-custodial bitcoin into their apps and services.
It eliminates the need for third parties, simplifies the complexities of Bitcoin and Lightning, and enables seamless onboarding for billions of users to the future of value transfer.

**The Breez SDK is free for developers.**


## **What is the Breez SDK - Nodeless *(Spark Implementation)*?**

It’s a nodeless integration that offers a self-custodial, end-to-end solution for integrating bitcoin, utilizing the Bitcoin-native Layer 2 Lightning & Spark with on-chain interoperability. Using the SDK you'll able to:

- Send payments via various protocols such as: Bolt11, LNURL-Pay, Lightning address, BTC address, Spark address, BTKN
- Receive payments via various protocols such as: Bolt11, BTC address, Spark address, BTKN

**Key Features**
- [x] Send and receive Lightning payments
- [x] Send and receive via LNURL-pay & Lightning addresses 
- [x] Send and receive Spark payments (BTC)
- [x] Issue, send and receive Spark tokens (BTKN)
- [x] On-chain interoperability
- [x] Bindings to all popular languages & frameworks
- [x] Keys are only held by users
- [x] Multi-app & multi-device support via real-time sync service 
- [x] Payments persistency including restore support
- [x] Automatic claims
- [x] WebAssembly support
- [x] Free open-source solution

## Getting Started 

Head over to the [Breez SDK documentation](https://sdk-doc-spark.breez.technology/) to start implementing bitcoin in your app.

You'll need an API key to use the Breez SDK. To request an API key is free - you just need to [complete this simple form](https://breez.technology/request-api-key/#contact-us-form-sdk).


## **API**

API documentation is [here](https://breez.github.io/spark-sdk/breez_sdk_spark/index.html).


## **Command Line**

The [Breez SDK - Nodeless *(Spark Implementation)* cli](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/cli) is a command line client that allows you to interact with and test the functionality of the SDK.

## Demo

Looking for a quick way to try the SDK in the browser or as PWA? Check out our demo app *Glow*:

- **Live demo:** [https://glow-app.co](https://glow-app.co)
- **Repo:** [breez/breez-sdk-spark-example](https://github.com/breez/breez-sdk-spark-example)  

> **Note:** The demo is for demonstration purposes only and not intended for production use.

## **Support**

Have a question for the team? Join our [Telegram channel](https://t.me/breezsdk) or email us at [contact@breez.technology](mailto:contact@breez.technology)
 

## How Does Nodeless *(Spark Implementation)* Work?

The Breez SDK uses Spark, a Bitcoin-native Layer 2 built on a shared signing protocol, to enable real-time, low-fee, self-custodial payments.

When sending a payment, Spark delegates the transfer of on-chain bitcoin to the recipient through a multi-signature process.
Spark Operators help facilitate the transfer, but they cannot move funds without the user. This allows the payment to settle almost instantly, without requiring a blockchain confirmation.

When receiving a payment, the same process works in reverse: the network updates ownership of the bitcoin to the user through the shared signing system, recording the change off-chain while always keeping the funds secure.

Unlike blockchains, rollups, or smart contracts, Spark doesn’t create a new ledger or require trust in external consensus.
On Bitcoin’s main chain, Spark transactions appear as a series of multi-sig wallets. Off-chain, Spark keeps a lightweight record of balances and history.

Funds are self-custodial: you can exit Spark at any time and reclaim your bitcoin directly on the Bitcoin main chain.


## **Build & Test**

- **crates**: Contains the root Rust cargo workspace.
    - **breez-sdk**: Collection of Breez SDK crates.
        - **bindings**: The FFI bindings for Go, Kotlin, Python, React Native, and Swift.
        - **cli**: Contains the Rust command line interface client for the SDK.
        - **common**: The common Breez SDK Rust library.
        - **core**: The core Breez SDK Rust library.
        - **wasm**: The Wasm interface bindings.        
    - **spark**: The Spark crate.
- **packages**: Contains the packages for Flutter, React Native and Wasm.


## **Contributing**

Contributions are always welcome. Please read our [contribution guide](CONTRIBUTING.md) to get started.


## **SDK Development Roadmap**

- [x] Send/Receive Lightning payments
- [x] Send/Receive Spark payments
- [x] Send/Receive via on-chain addresses
- [x] CLI Interface
- [x] Go, C#, React Native, Python, JS, Flutter, Kotlin & Swift languages bindings
- [x] WebAssembly support
- [x] Send via LNURL-Pay
- [x] Send to a Lightning address
- [x] Payments persistency including restore support
- [x] Automatic on-chain claims 
- [x] Receive via LNURL-Pay w/ offline & Lightning address support
- [x] Full support (issue, send & receive) for Spark tokens (BTKN)
- [x] LNURL-Withdraw
- [x] Sign and verify arbitrary messages 
- [x] Real-time sync
- [x] External input parsers
- [x] External signer
- [ ] Seedless restore
- [ ] NWC
- [ ] WebLN
- [ ] LNURL-Auth
- [ ] Fiat on-ramp
- [ ] BTC <> USDX swaps
- [ ] Hodl invoice support 
- [ ] Bolt12
- [ ] Add additional fees via a dedicated portal



