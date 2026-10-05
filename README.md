# Breez SDK - Spark

## **Overview**

The Breez SDK provides developers with an end-to-end solution for integrating instant, non-custodial bitcoin and stablecoins into their apps and services.
It eliminates the need for third parties, simplifies the complexities of Bitcoin and Lightning, and enables seamless onboarding for billions of users to the future of value transfer.

**The Breez SDK is free for developers.**

## **What is the Breez SDK - Spark?**

It’s a nodeless integration that offers a non-custodial, end-to-end solution for integrating bitcoin and stablecoins, utilizing the Bitcoin-native Layer 2 Lightning & Spark, with on-chain interoperability. Using the Breez SDK, you’ll be able to:

- Send and receive bitcoin via Lightning addresses, Bolt11 invoices, LNURL-Pay, bitcoin addresses, and Spark addresses
- Send and receive USDC or USDT on Ethereum, Base, Arbitrum, Solana, Tron, and other networks, to and from a bitcoin or USD balance
- Issue, send, and receive Spark tokens (BTKN)

## **Key Features**

<table>
<tr>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/lnurl.html"><b>Fully-featured Lightning</b></a><br>Lightning addresses, invoices, LNURL-Pay, -Withdraw, -Auth, and -Verify.</td>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/install.html"><b>Languages &amp; Frameworks</b></a><br>Kotlin, Swift, JS, React Native, Flutter, Go, Python, C#, and WASM.</td>
</tr>
<tr>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/passkey.html"><b>Passkey Login</b></a><br>Seedless onboarding and restore, no recovery phrase needed.</td>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/cross_chain.html"><b>USDT &amp; USDC</b></a><br>Users can send/receive stablecoins from their BTC or stable balance.</td>
</tr>
<tr>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/cash_app_to_usdc_usdt.html"><b>Cash App to USDC/USDT</b></a><br>Users pay from Cash App and the recipient gets USDC or USDT on their chain.</td>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/stable_balance.html"><b>Stable Balance</b></a><br>Users can hold their balance in USD to avoid BTC price volatility.</td>
</tr>
<tr>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/onchain_claims.html#instant-expedited-claims"><b>Instant Deposits</b></a><br>Claim on-chain deposits in seconds, no confirmations needed.</td>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/onchain_claims.html"><b>Automatic Claims</b></a><br>On-chain deposits are claimed automatically within the fee limits you set.</td>
</tr>
<tr>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/config.html#real-time-sync-server-url"><b>Multi-device/app Sync</b></a><br>Users can access their balance from multiple devices and apps.</td>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/contacts.html"><b>Contacts API</b></a><br>Save and reuse Lightning addresses, synced across devices.</td>
</tr>
<tr>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/customizing.html#with-storage"><b>Caching &amp; Persistence</b></a><br>Out-of-the-box caching and persistence for a smooth UX.</td>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/tokens.html"><b>Spark Tokens</b></a><br>Issue, send, receive, and convert Spark tokens (BTKN).</td>
</tr>
<tr>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/external_signer.html"><b>External Signer</b></a><br>Bring your own key management.</td>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/turnkey.html"><b>Turnkey Signer</b></a><br>A built-in signer for server-side, non-custodial embedded wallets.</td>
</tr>
<tr>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/buy_bitcoin.html"><b>Integrated On-ramps</b></a><br>Let users buy bitcoin directly via Cash App and MoonPay.</td>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/fiat_currencies.html"><b>Fiat Currencies</b></a><br>Real-time exchange rates to display value in fiat currencies.</td>
</tr>
<tr>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/server_mode.html"><b>Multi-user Server Mode</b></a><br>Run multiple user balances from a single backend server.</td>
<td width="50%" valign="top"><a href="https://sdk-doc-spark.breez.technology/guide/unilateral_exit.html"><b>Unilateral Exit</b></a><br>Users can always withdraw to the Bitcoin blockchain without the Spark operators.</td>
</tr>
<tr>
<td width="50%" valign="top"><a href="https://github.com/breez/spark-sdk"><b>Open-source</b></a><br>Free and open-source, available to any developer at any scale.</td>
<td width="50%" valign="top"><a href="https://partners.breez.technology/"><b>Partner Portal</b></a><br>Track your volume and activity in one dashboard.</td>
</tr>
</table>

## Getting Started 

Head over to the [Breez SDK documentation](https://sdk-doc-spark.breez.technology/) to start integrating instant bitcoin and stablecoins into your app or service.

You'll need an API key to use the Breez SDK. It's free: [complete this simple form to request one](https://breez.technology/request-api-key/#contact-us-form-sdk).


## **API**

API documentation is [here](https://breez.github.io/spark-sdk/breez_sdk_spark/index.html).


## **Command Line**

The [Breez SDK - Spark cli](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/cli) is a command line client that allows you to interact with and test the functionality of the SDK.

## Demo

Looking for a quick way to try the Breez SDK in the browser or as PWA? Check out our demo app *Glow*:

- **Live demo:** [https://glow-app.co](https://glow-app.co)
- **Repo:** [breez/glow-web](https://github.com/breez/glow-web)  

> **Note:** The demo is for demonstration purposes only and not intended for production use.

## **Example Apps**

The repository includes full working CLI example apps for every supported language, demonstrating end-to-end SDK usage.

| Language | CLI Example App |
|----------|-----------------|
| Rust | [crates/breez-sdk/cli](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/cli) |
| JavaScript/TypeScript | [cli/langs/wasm](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/wasm), [glow-web](https://github.com/breez/glow-web) |
| Swift | [cli/langs/swift](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/swift) |
| Kotlin | [cli/langs/kotlin-multiplatform](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/kotlin-multiplatform) |
| Flutter/Dart | [cli/langs/flutter](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/flutter) |
| Python | [cli/langs/python](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/python) |
| Go | [cli/langs/golang](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/golang) |
| React Native | [cli/langs/react-native](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/react-native) |
| C# | [cli/langs/csharp](https://github.com/breez/spark-sdk/tree/main/crates/breez-sdk/bindings/examples/cli/langs/csharp) |

## **Support**

Have a question for the team? Join our [Telegram channel](https://t.me/breezsdk) or email us at [contact@breez.technology](mailto:contact@breez.technology)
 

## How does the Breez SDK - Spark work?

The Breez SDK uses Spark, a Bitcoin-native Layer 2 built on a shared signing protocol, to enable real-time, low-fee, self-custodial payments.

When sending a payment, Spark delegates the transfer of on-chain bitcoin to the recipient through a multi-signature process.
Spark Operators help facilitate the transfer, but they cannot move funds without the user. This allows the payment to settle almost instantly, without requiring a blockchain confirmation.

When receiving a payment, the same process works in reverse: the network updates ownership of the bitcoin to the user through the shared signing system, recording the change off-chain while always keeping the funds secure.

Unlike blockchains, rollups, or smart contracts, Spark doesn’t create a new ledger or require trust in external consensus.
On Bitcoin’s main chain, Spark transactions appear as a series of multi-sig wallets. Off-chain, Spark keeps a lightweight record of balances and history.

Funds are non-custodial: you can exit Spark at any time and reclaim your bitcoin directly on the Bitcoin main chain.


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
- [x] LNURL-Auth
- [x] Fiat on-ramp
- [x] BTC <> USDB swaps
- [x] Hodl invoice support
- [x] Passkey login for seedless experience
- [x] Contacts management 
- [x] Stable balance
- [x] Multi-user server mode
- [x] USDT send support
- [x] USDC send support
- [x] Partner portal analytics
- [x] Unilateral exit
- [x] Cash App to USDC/USDT
- [x] USDT receive support
- [x] USDC receive support
- [x] Detect pending mempool deposits
- [x] Instant claim of on-chain deposits
- [ ] Add additional fees via the partner portal
- [ ] Token allowance
- [ ] Delegated spend
- [ ] NWC
- [ ] Bolt12




